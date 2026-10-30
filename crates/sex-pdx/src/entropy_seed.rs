// SPDX-License-Identifier: GPL-3.0-only
//! Finite private seed custody on an existing held SexNet route. Scalar
//! selectors, sequence numbers and tickets never create source authority.
pub use crate::entropy_source::Identity;
pub use crate::network_job::{MAX_SEQ,MAX_TICKET,INVALID,DENIED,STALE,BUSY,OFFLINE,TIMEOUT,IO,EXHAUSTED};
pub const OP: u64 = 0x20E;
pub const SEED_BYTES: usize = 32;
pub const READ_BYTES: usize = 4;
pub const BEGIN: u8 = 1;
pub const STATUS: u8 = 4;
pub const READ: u8 = 5;
pub const CANCEL: u8 = 6;
pub const RELEASE: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub seq: u32, pub kind: u8, pub offset: u16, pub length: u16,
    pub ticket: u32, pub a: u64,
}
impl Request {
    /// Preserve the network-job register layout without permitting seed
    /// request encoders to silently alias a noncanonical sequence or ticket.
    pub fn encode(self) -> [u64; 3] {
        assert!(self.valid(),"ENTROPY_SEED_CANONICAL_REQUEST");
        crate::network_job::Request {seq:self.seq,kind:self.kind,offset:self.offset,
            length:self.length,ticket:self.ticket,a:self.a}.encode()
    }
    pub fn decode(args: [u64; 3]) -> Result<Self, u32> {
        let r=crate::network_job::Request::decode(args)?;
        let request=Self {seq:r.seq,kind:r.kind,offset:r.offset,length:r.length,ticket:r.ticket,a:r.a};
        if request.valid() {Ok(request)}else{Err(INVALID)}
    }
    pub fn valid(self) -> bool {
        if self.seq==0 || self.seq>MAX_SEQ || self.a!=0 {return false;}
        if self.kind==BEGIN {
            return self.ticket==0 && self.offset==0 && self.length as usize==SEED_BYTES;
        }
        if self.ticket==0 || self.ticket>MAX_TICKET {return false;}
        match self.kind {
            STATUS|CANCEL|RELEASE => self.offset==0 && self.length==0,
            READ => self.length!=0 && self.length as usize<=READ_BYTES
                && (self.offset as usize)<SEED_BYTES
                && self.offset as usize+self.length as usize<=SEED_BYTES,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State { Collecting=1, Ready=2, Failed=3, Cancelled=4 }
impl State {
    pub fn decode(value:u8)->Option<Self> {
        Some(match value {1=>Self::Collecting,2=>Self::Ready,3=>Self::Failed,4=>Self::Cancelled,_=>return None})
    }
}
pub const fn status(state:State,produced:usize)->u32 {
    assert!(produced<=SEED_BYTES,"ENTROPY_SEED_STATUS_BOUND");
    ((state as u32)<<24)|produced as u32
}
pub fn decode_status(value:u32)->Option<(State,usize)> {
    let state=State::decode((value>>24)as u8)?;let produced=(value&MAX_SEQ)as usize;
    if produced>SEED_BYTES {None}else{Some((state,produced))}
}
pub const fn reply(seq:u32,value:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"ENTROPY_SEED_REPLY_SEQUENCE");
    (0x7Au64<<56)|((seq as u64)<<32)|value as u64
}
pub const fn error(seq:u32,code:u32)->u64 {
    assert!(seq!=0&&seq<=MAX_SEQ,"ENTROPY_SEED_REPLY_SEQUENCE");
    (0xEAu64<<56)|((seq as u64)<<32)|code as u64
}
pub fn decode_reply(word:u64,seq:u32)->Result<u32,u32> {
    if seq==0||seq>MAX_SEQ {return Err(INVALID);}
    if ((word>>32)as u32&MAX_SEQ)!=seq {return Err(STALE);}
    match word>>56 {0x7A=>Ok(word as u32),0xEA=>Err(word as u32),_=>Err(INVALID)}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn begin()->Request {Request {seq:1,kind:BEGIN,offset:0,length:32,ticket:0,a:0}}
    fn held(kind:u8)->Request {Request {seq:2,kind,offset:0,length:0,ticket:1,a:0}}
    fn raw(request:Request)->[u64;3] {
        crate::network_job::Request {seq:request.seq,kind:request.kind,offset:request.offset,
            length:request.length,ticket:request.ticket,a:request.a}.encode()
    }
    #[test]
    fn seed_request_uses_exact_network_layout_and_roundtrips_only_seed_operations() {
        assert_eq!(begin().encode(),[1|(1<<24)|(32<<48),0,0]);
        for request in [begin(),held(STATUS),held(CANCEL),held(RELEASE),Request {kind:READ,length:4,offset:28,..held(READ)}] {
            assert!(request.valid());assert_eq!(Request::decode(request.encode()),Ok(request));assert_eq!(request.encode(),raw(request));
        }
    }
    #[test]
    fn constructed_and_raw_begin_lengths_tickets_reserved_data_and_kinds_are_checked() {
        for request in [Request {seq:0,..begin()},Request {length:0,..begin()},Request {length:31,..begin()},Request {length:33,..begin()},Request {offset:1,..begin()},Request {ticket:1,..begin()},Request {a:1,..begin()}] {
            assert!(!request.valid());assert_eq!(Request::decode(raw(request)),Err(INVALID));
        }
        for kind in [0,2,3,8,9,255] {let request=Request {kind,..held(kind)};assert!(!request.valid());assert_eq!(Request::decode(raw(request)),Err(INVALID));}
        for request in [Request {seq:MAX_SEQ+1,..begin()},Request {seq:u32::MAX,..begin()}] {
            assert!(!request.valid());assert!(std::panic::catch_unwind(||request.encode()).is_err());
        }
        for alias in [[begin().encode()[0],1<<32,0],[begin().encode()[0],0,1],[begin().encode()[0],u64::MAX,0]] {assert_eq!(Request::decode(alias),Err(INVALID));}
    }
    #[test]
    fn held_operations_require_finite_ticket_and_canonical_empty_arguments() {
        for kind in [STATUS,CANCEL,RELEASE] {
            for request in [Request {ticket:0,..held(kind)},Request {ticket:MAX_TICKET+1,..held(kind)},Request {offset:1,..held(kind)},Request {length:1,..held(kind)},Request {a:1,..held(kind)}] {
                assert!(!request.valid());assert_eq!(Request::decode(raw(request)),Err(INVALID));
            }
            let request=Request {ticket:MAX_TICKET,seq:MAX_SEQ,..held(kind)};assert_eq!(Request::decode(request.encode()),Ok(request));
        }
    }
    #[test]
    fn every_read_extent_is_bounded_to_one_through_four_of_the_exact_thirty_two_bytes() {
        for offset in 0..=33u16 {for length in 0..=5u16 {
            let request=Request {kind:READ,offset,length,..held(READ)};
            let valid=offset<32&&(1..=4).contains(&length)&&offset+length<=32;
            assert_eq!(request.valid(),valid);
            assert_eq!(Request::decode(raw(request)).is_ok(),valid);
        }}
        for request in [Request {kind:READ,length:4,a:1,..held(READ)},Request {kind:READ,length:4,ticket:0,..held(READ)},Request {kind:READ,length:4,ticket:MAX_TICKET+1,..held(READ)}] {assert_eq!(Request::decode(raw(request)),Err(INVALID));}
    }
    #[test]
    fn status_has_only_four_states_and_never_masks_excess_production() {
        for state in [State::Collecting,State::Ready,State::Failed,State::Cancelled] {for produced in 0..=SEED_BYTES {assert_eq!(decode_status(status(state,produced)),Some((state,produced)));}}
        for value in [0,5<<24,255<<24,(1<<24)|33,(2<<24)|0xFFFFFF] {assert_eq!(decode_status(value),None);}
        assert!(std::panic::catch_unwind(||status(State::Collecting,33)).is_err());
        assert!(std::panic::catch_unwind(||status(State::Collecting,usize::MAX)).is_err());
    }
    #[test]
    fn exact_reply_tags_sequences_and_all_thirty_two_payload_bits_are_preserved() {
        for seq in [1,MAX_SEQ] {for value in [0,1,u32::MAX] {
            assert_eq!(decode_reply(reply(seq,value),seq),Ok(value));assert_eq!(decode_reply(error(seq,value),seq),Err(value));
        }}
        assert_eq!(decode_reply(reply(1,0),2),Err(STALE));
        for seq in [0,MAX_SEQ+1,u32::MAX] {
            assert_eq!(decode_reply(reply(1,0),seq),Err(INVALID));
            assert!(std::panic::catch_unwind(||reply(seq,0)).is_err());assert!(std::panic::catch_unwind(||error(seq,IO)).is_err());
        }
        for tag in [0,0x78,0x79,0xE8,0xE9,0xFF] {assert_eq!(decode_reply(((tag as u64)<<56)|(1<<32),1),Err(INVALID));}
    }
}
