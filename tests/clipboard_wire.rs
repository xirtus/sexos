// SPDX-License-Identifier: GPL-3.0-only
//! Actual scalar ABI canonicalization, without transport or authority mocks.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path = "../crates/sex-pdx/src/text_encoding.rs"] pub mod text_encoding;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path = "../crates/sex-pdx/src/text_selection.rs"] pub mod text_selection;
#[path = "../crates/sex-pdx/src/clipboard.rs"] pub mod clipboard;
use clipboard::*;
use text_selection::Source;
const SOURCE:Source=Source{generation:7,revision:11};
fn request(r:Request){let p=r.wire();assert_eq!(Request::decode(p.opcode,p.args),Some(r));}
fn response(r:Response){let p=r.wire();assert_eq!(Response::decode(p.opcode,p.args),Some(r));}

#[test]fn complete_request_response_roundtrips_at_finite_boundaries(){
    for ticket in [1,u32::MAX]{
        for len in [1,16,MAX_BYTES]{request(Request::Begin{ticket,len,source:SOURCE});request(Request::Publish{ticket,len,hash:u64::MAX});}
        for r in [Request::Read{ticket},Request::Get{ticket,offset:MAX_BYTES-16},Request::Finish{ticket,generation:u32::MAX,hash:u64::MAX},Request::Cancel{ticket},Request::Query{ticket}]{request(r);}
        for action in [Action::Copy,Action::Cut,Action::Paste]{response(Response::Intent{ticket,action});}
        for phase in [Phase::Begin,Phase::Publish,Phase::Read,Phase::Finish,Phase::Cancel,Phase::Query]{for status in [Status::Ok,Status::Busy,Status::Denied,Status::Invalid,Status::Empty,Status::Invalidated,Status::Exhausted,Status::Uncertain]{
            response(Response::Meta{ticket,phase,status,len:0,generation:0,hash:0});
        }}response(Response::Meta{ticket,phase:Phase::Read,status:Status::Ok,len:MAX_BYTES,generation:u32::MAX,hash:u64::MAX});
    }
}
#[test]fn actual_catalog_matrix_grants_only_four_roles_and_never_infers_task_from_pd(){
    for pd in 0..=20{for surface in [156,300,301,307,999]{for task in [0,1,123]{for coordinator in [0,3,9]{
        let c=app_lifecycle::CatalogInfo{target_pd:pd,task_id:task,surface_id:surface,coordinator_pd:coordinator};let p=Participant::from_catalog(c);
        let expected=task!=0&&coordinator==3&&matches!((pd,surface),(9,156)|(15,300)|(16,301)|(18,307));assert_eq!(p.is_some(),expected);
        if let Some(p)=p{assert_eq!(p.subject,Subject{pd,task});assert!(p.allows(Action::Copy));assert_eq!(p.allows(Action::Cut),pd!=15);assert_eq!(p.allows(Action::Paste),pd!=15);assert_eq!(p.route,match pd{9=>41,15=>43,16=>46,18=>56,_=>unreachable!()});}
    }}}}
}
#[test]fn request_scalar_width_reserved_word_and_source_failures_are_rejected(){
    let begin=Request::Begin{ticket:1,len:16,source:SOURCE}.wire();
    for args in [[0,7,11],[1,0,11],[1|(0u64<<32),7,11],[1|((MAX_BYTES+1)as u64)<<32,7,11],[begin.args[0]|1u64<<48,7,11]]{assert!(Request::decode(BEGIN,args).is_none(),"{args:?}");}
    for op in [PUBLISH,READ,FINISH,CANCEL,QUERY]{for ticket in [0,u32::MAX as u64+1,u64::MAX]{assert!(Request::decode(op,[ticket,if op==FINISH{1}else if op==PUBLISH{16}else{0},0]).is_none());}}
    for op in [READ,CANCEL,QUERY]{for args in [[1,1,0],[1,0,1],[1,u64::MAX,u64::MAX]]{assert!(Request::decode(op,args).is_none());}}
    for args in [[1,0,0],[1,MAX_BYTES as u64+1,0]]{assert!(Request::decode(PUBLISH,args).is_none());}
    for args in [[1,0,0],[1,u32::MAX as u64+1,0]]{assert!(Request::decode(FINISH,args).is_none());}
    for args in [[1|1u64<<32,0,0],[1|(MAX_BYTES as u64)<<32,0,0],[1|1u64<<48,0,0],[1,1,0],[1,0,1]]{assert!(Request::decode(GET,args).is_none());}
}
#[test]fn every_chunk_offset_count_and_tail_is_canonical_for_both_directions(){
    for offset in (0..MAX_BYTES).step_by(16){for count in 1..=16{let mut bytes=[0;16];bytes[..count].fill(b'x');request(Request::Write{ticket:u32::MAX,offset,count,bytes});response(Response::Data{ticket:1,offset,count,bytes});}}
    for count in 1..16{for tail in count..16{let mut bytes=[0;16];bytes[..count].fill(b'x');bytes[tail]=1;let w=Request::Write{ticket:1,offset:0,count,bytes}.wire();assert!(Request::decode(WRITE,w.args).is_none());assert!(Response::decode(DATA,w.args).is_none());}}
    let base=Request::Write{ticket:1,offset:0,count:16,bytes:[b'x';16]}.wire();
    for meta in [0,1,1|17u64<<48,1|1u64<<32|16u64<<48,1|(MAX_BYTES as u64)<<32|16u64<<48,base.args[0]|1u64<<56]{assert!(Request::decode(WRITE,[meta,base.args[1],base.args[2]]).is_none());assert!(Response::decode(DATA,[meta,base.args[1],base.args[2]]).is_none());}
}
#[test]fn response_metadata_reserved_bits_bad_status_and_failure_payload_are_rejected(){
    let good=Response::Meta{ticket:1,phase:Phase::Read,status:Status::Ok,len:16,generation:1,hash:3}.wire();
    for bit in 48..64{let mut a=good.args;a[0]|=1u64<<bit;assert!(Response::decode(REPLY,a).is_none());}
    for bit in 16..32{let mut a=good.args;a[1]|=1u64<<bit;assert!(Response::decode(REPLY,a).is_none());}
    for phase in [0,7,255]{assert!(Response::decode(REPLY,[1|phase<<32,0,0]).is_none());}
    for status in [8,255]{assert!(Response::decode(REPLY,[1|1u64<<32|status<<40,0,0]).is_none());}
    for field in 0..3{let mut a=[1|(Phase::Publish as u64)<<32|(Status::Invalid as u64)<<40,0,0];if field==0{a[1]=1}else if field==1{a[1]=1u64<<32}else{a[2]=1}assert!(Response::decode(REPLY,a).is_none());}
    assert!(Response::decode(REPLY,[good.args[0],MAX_BYTES as u64+1,0]).is_none());assert!(Response::decode(INTENT,[0,1,0]).is_none());assert!(Response::decode(INTENT,[u32::MAX as u64+1,1,0]).is_none());
    for args in [[1,0,0],[1,4,0],[1,1,1]]{assert!(Response::decode(INTENT,args).is_none());}
}
#[test]fn opcode_sets_do_not_alias_requests_with_receipts_and_packet_erase_clears_all_words(){
    for op in 0..=0x300{assert_eq!(request_opcode(op),[BEGIN,WRITE,PUBLISH,READ,GET,FINISH,CANCEL,QUERY].contains(&op));assert_eq!(response_opcode(op),[INTENT,REPLY,DATA].contains(&op));assert!(!(request_opcode(op)&&response_opcode(op)));}
    let mut packet=Packet{opcode:u64::MAX,args:[u64::MAX;3]};packet.erase();assert_eq!(packet,Packet{opcode:0,args:[0;3]});assert!(text(b"ASCII\n !~"));assert!(!text(b"a\rb"));assert!(!text(b"a\0b"));assert!(!text(&[255]));assert_eq!(hash(b""),0xcbf29ce484222325);
}

#[test] fn whole_utf8_text_profile_is_independent_of_opaque_register_chunk_boundaries(){
    assert!(text("é € 𐍈\n\t".as_bytes()));
    for scalar in ["é","€","𐍈"]{for split in 1..scalar.len(){
        let mut body=vec![b'a';32];let start=16-split;body[start..start+scalar.len()].copy_from_slice(scalar.as_bytes());
        assert!(text(&body));assert!(!text(&body[..16]));
        for (index,chunk)in body.chunks(16).enumerate(){let mut bytes=[0;16];bytes[..chunk.len()].copy_from_slice(chunk);
            request(Request::Write{ticket:1,offset:index*16,count:chunk.len(),bytes});response(Response::Data{ticket:1,offset:index*16,count:chunk.len(),bytes});
        }
    }}
    for bad in [&b"\xc0\xaf"[..],&b"\xe2\x82"[..],&b"\xed\xa0\x80"[..],&b"\xf4\x90\x80\x80"[..],&b"\xc2\x80"[..],&b"\x1b"[..]]{assert!(!text(bad));}
}
