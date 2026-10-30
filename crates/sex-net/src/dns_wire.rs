// SPDX-License-Identifier: GPL-3.0-only
//! Narrow, allocation-free DNS A codec for the canonical `probe.sexos.test`.
//! This module carries protocol data only. It grants no endpoint or app authority.
//! Supported replies bind one case-insensitive canonical A/IN question, at most
//! eight answer records and eight combined authority/additional records. Bounded
//! registered-boundary backward compression and up to four CNAME links are
//! supported. Extra sections accept only IN A/CNAME/NS/SOA/AAAA with known RDATA
//! structure; they never supply the selected address. No EDNS, TCP fallback,
//! DNSSEC/authenticity, caching or endpoint authority is established.
//! UDP source/destination endpoint binding belongs to the private transport job.

pub const MAX_BYTES: usize = 512;
pub const MAX_ANSWERS: usize = 8;
pub const QUERY_BYTES: usize = 12+sex_net_config::DNS_QUESTION.len();
const NAME: &[u8] = sex_net_config::DNS_NAME;
const QUESTION: &[u8] = sex_net_config::DNS_QUESTION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer { pub address: [u8; 4], pub ttl: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidId,
    Oversized,
    Truncated,
    Malformed,
    Unsupported,
    Rcode(u8),
    NoAnswer,
}

/// Emit one standard recursive A/IN question. Bytes beyond the returned length
/// remain outside the datagram. A rejected zero ID leaves the buffer unchanged.
pub fn query(id: u16, out: &mut [u8; MAX_BYTES]) -> Result<usize, Error> {
    if id == 0 { return Err(Error::InvalidId); }
    out[..QUERY_BYTES].fill(0);
    out[..2].copy_from_slice(&id.to_be_bytes());
    out[2] = 1; // RD; opcode QUERY, not a response.
    out[5] = 1; // QDCOUNT=1.
    out[12..QUERY_BYTES].copy_from_slice(QUESTION);
    Ok(QUERY_BYTES)
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], Error> {
    let end = cursor.checked_add(length).ok_or(Error::Malformed)?;
    let data = bytes.get(*cursor..end).ok_or(Error::Truncated)?;
    *cursor = end;
    Ok(data)
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let mut cursor = offset;
    let data = take(bytes, &mut cursor, 2)?;
    Ok(u16::from_be_bytes([data[0], data[1]]))
}
// RFC1035 §§3.3.1/4.1.4: names and CNAME RDATA use label sequences and
// compression. RFC2181 §§5/8/10: RRset TTL minimum, high-bit TTL→zero and
// exclusive CNAME ownership. https://www.rfc-editor.org/rfc/rfc1035.html
// https://www.rfc-editor.org/rfc/rfc2181.html
const MAX_EXTRA: usize = 8;
const MAX_RECORDS: usize = MAX_ANSWERS + MAX_EXTRA;
const MAX_NAME: usize = 255;
const MAX_HOPS: usize = 16;
const MAX_CHAIN: usize = 4;
#[derive(Clone, Copy)]
struct Name { bytes: [u8; MAX_NAME], length: usize }
impl Name {
    fn new() -> Self { Self { bytes: [0; MAX_NAME], length: 0 } }
    fn same(&self, other: &Self) -> bool {
        self.bytes[..self.length] == other.bytes[..other.length]
    }
}
struct Boundaries([u8; MAX_BYTES / 8]);
impl Boundaries {
    fn has(&self, offset: usize) -> bool {
        offset < MAX_BYTES && self.0[offset / 8] & (1 << (offset % 8)) != 0
    }
    fn insert(&mut self, offset: usize) { self.0[offset / 8] |= 1 << (offset % 8); }
}
/// Decode into one reused scratch. Only boundaries of *completed* prior names
/// are registered; field bytes, label payload and the current name are excluded.
/// Each physical pointer is backward, each name has ≤16 jumps and ≤255 expanded
/// wire bytes. The loop has a separate255+16 step cap, independent of input.
fn name(bytes: &[u8], start: usize, known: &mut Boundaries, register: bool, out: &mut Name) -> Result<usize, Error> {
    out.length = 0;
    let mut cursor = start;
    let mut end = None;
    let mut hops = 0;
    for _ in 0..MAX_NAME + MAX_HOPS {
        let tag = *bytes.get(cursor).ok_or(Error::Truncated)?;
        match tag & 0xC0 {
            0xC0 => {
                let target = (u16_at(bytes, cursor)? & 0x3FFF) as usize;
                if target >= cursor || !known.has(target) { return Err(Error::Unsupported); }
                hops += 1;
                if hops > MAX_HOPS { return Err(Error::Unsupported); }
                if end.is_none() { end = Some(cursor + 2); }
                cursor = target;
            },
            0 => {
                let length = tag as usize;
                let expanded = out.length.checked_add(1 + length).ok_or(Error::Malformed)?;
                if expanded > MAX_NAME { return Err(Error::Malformed); }
                let mut next = cursor + 1;
                let label = take(bytes, &mut next, length)?;
                out.bytes[out.length] = tag;
                for (target, source) in out.bytes[out.length+1..expanded].iter_mut().zip(label) {
                    *target = source.to_ascii_lowercase();
                }
                out.length = expanded;
                cursor = next;
                if length == 0 {
                    let end = end.unwrap_or(cursor);
                    if register {
                        let mut physical = start;
                        while physical < end {
                            known.insert(physical);
                            let length = bytes[physical];
                            if length == 0 || length & 0xC0 == 0xC0 { break; }
                            physical += 1 + length as usize;
                        }
                    }
                    return Ok(end);
                }
            },
            _ => return Err(Error::Malformed),
        }
    }
    Err(Error::Malformed)
}
#[derive(Clone, Copy)]
struct Record { owner: u16, data: u16, kind: u16, ttl: u32 }
impl Record { const EMPTY: Self = Self { owner: 0, data: 0, kind: 0, ttl: 0 }; }
fn ttl(bytes: &[u8]) -> u32 {
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if value & 0x8000_0000 != 0 { 0 } else { value }
}
fn read_record(bytes: &[u8], cursor: &mut usize, known: &mut Boundaries, answer: bool, scratch: &mut Name) -> Result<Record, Error> {
    let owner = *cursor;
    *cursor = name(bytes, owner, known, true, scratch)?;
    let fields = take(bytes, cursor, 10)?;
    let kind = u16::from_be_bytes([fields[0], fields[1]]);
    if fields[2..4] != [0, 1] { return Err(Error::Unsupported); }
    let length = u16::from_be_bytes([fields[8], fields[9]]) as usize;
    let data = *cursor;
    let end = data.checked_add(length).ok_or(Error::Malformed)?;
    if kind == 1 && length != 4 || kind == 28 && !answer && length != 16 { return Err(Error::Malformed); }
    if end > bytes.len() { return Err(Error::Truncated); }
    match kind {
        1 => if length != 4 { return Err(Error::Malformed); },
        5 | 2 if kind == 5 || !answer => {
            if name(bytes, data, known, true, scratch)? != end { return Err(Error::Malformed); }
        },
        6 if !answer => { // SOA: two domain names and five32-bit integers.
            let next = name(bytes, data, known, true, scratch)?;
            let next = name(bytes, next, known, true, scratch)?;
            if next.checked_add(20) != Some(end) { return Err(Error::Malformed); }
        },
        28 if !answer => if length != 16 { return Err(Error::Malformed); },
        _ => return Err(Error::Unsupported),
    }
    *cursor = end;
    Ok(Record { owner: owner as u16, data: data as u16, kind, ttl: ttl(&fields[4..8]) })
}
/// Check aliases across all accepted sections, including unrelated names. Each
/// pair is visited once; duplicate CNAMEs may agree but cannot conflict or own
/// any other supported data (RFC2181§10; DNSSEC exception types unsupported).
/// Every alias graph walk is ≤4 links, each scan ≤16 records. No recursion, heap,
/// per-record full-name copies or unbounded name/pointer traversal is used.
fn validate_aliases(bytes: &[u8], records: &[Record], known: &mut Boundaries, left: &mut Name, right: &mut Name) -> Result<(), Error> {
    for (index, a) in records.iter().enumerate() {
        for b in &records[index+1..] {
            if a.kind != 5 && b.kind != 5 { continue; }
            name(bytes, a.owner as usize, known, false, left)?;
            name(bytes, b.owner as usize, known, false, right)?;
            if !left.same(right) { continue; }
            if a.kind != b.kind { return Err(Error::Malformed); }
            name(bytes, a.data as usize, known, false, left)?;
            name(bytes, b.data as usize, known, false, right)?;
            if !left.same(right) { return Err(Error::Malformed); }
        }
    }
    for record in records.iter().filter(|r| r.kind == 5) {
        name(bytes, record.owner as usize, known, false, left)?;
        let mut visited = [0u16; MAX_CHAIN + 1];
        visited[0] = record.owner;
        for depth in 0..=MAX_CHAIN {
            let mut target = None;
            for candidate in records.iter().filter(|r| r.kind == 5) {
                name(bytes, candidate.owner as usize, known, false, right)?;
                if left.same(right) { target = Some(candidate.data); break; }
            }
            let Some(target) = target else { break; };
            if depth == MAX_CHAIN { return Err(Error::Unsupported); }
            name(bytes, target as usize, known, false, right)?;
            core::mem::swap(left, right);
            for previous in &visited[..=depth] {
                name(bytes, *previous as usize, known, false, right)?;
                if left.same(right) { return Err(Error::Malformed); }
            }
            visited[depth+1] = target;
        }
    }
    Ok(())
}

/// Parse endpoint-bound data. Wrong ID or a well-formed unrelated question gives
/// None. A positive answer requires full validation of all sections and extent.
/// Only the exact question's answer-section CNAME chain can select an A RRset;
/// authority/additional data never supplies its address or extends authority.
pub fn parse(expected_id: u16, response: &[u8]) -> Result<Option<Answer>, Error> {
    if expected_id == 0 { return Err(Error::InvalidId); }
    if response.len() > MAX_BYTES { return Err(Error::Oversized); }
    if response.len() < 12 { return Err(Error::Truncated); }
    let id = u16_at(response, 0)?;
    if id == 0 { return Err(Error::InvalidId); }
    if id != expected_id { return Ok(None); }
    if u16_at(response, 4)? != 1 { return Err(Error::Malformed); }
    let mut known = Boundaries([0; MAX_BYTES / 8]);
    let mut current = Name::new();
    let mut scratch = Name::new();
    let mut cursor = name(response, 12, &mut known, true, &mut current)?;
    let question_tail = take(response, &mut cursor, 4)?;
    if current.bytes[..current.length] != *NAME || question_tail != [0, 1, 0, 1] { return Ok(None); }
    let flags = u16_at(response, 2)?;
    if flags & 0x8000 == 0 { return Err(Error::Malformed); }
    if flags & 0x7840 != 0 { return Err(Error::Unsupported); }
    if flags & 0x0200 != 0 { return Err(Error::Truncated); }
    if flags & 0x000F != 0 { return Err(Error::Rcode((flags & 15) as u8)); }
    let answers = u16_at(response, 6)? as usize;
    let authority = u16_at(response, 8)? as usize;
    let additional = u16_at(response, 10)? as usize;
    if answers > MAX_ANSWERS || authority + additional > MAX_EXTRA { return Err(Error::Unsupported); }
    let count = answers + authority + additional;
    let mut records = [Record::EMPTY; MAX_RECORDS];
    for (index, record) in records[..count].iter_mut().enumerate() {
        *record = read_record(response, &mut cursor, &mut known, index < answers, &mut scratch)?;
    }
    if cursor != response.len() { return Err(Error::Malformed); }
    validate_aliases(response, &records[..count], &mut known, &mut current, &mut scratch)?;
    // Alias validation reuses both scratch names; restart from the bound question.
    name(response, 12, &mut known, false, &mut current)?;
    let mut minimum = u32::MAX;
    for depth in 0..=MAX_CHAIN {
        let mut address = None;
        let mut alias = None;
        for record in &records[..answers] {
            name(response, record.owner as usize, &mut known, false, &mut scratch)?;
            if !current.same(&scratch) { continue; }
            minimum = minimum.min(record.ttl);
            if record.kind == 1 && address.is_none() {
                let data = &response[record.data as usize..record.data as usize+4];
                address = Some([data[0], data[1], data[2], data[3]]);
            } else if record.kind == 5 { alias = Some(record.data); }
        }
        if let Some(address) = address { return Ok(Some(Answer { address, ttl: minimum })); }
        let Some(alias) = alias else { return Err(Error::NoAnswer); };
        if depth == MAX_CHAIN { return Err(Error::Unsupported); }
        name(response, alias as usize, &mut known, false, &mut scratch)?;
        core::mem::swap(&mut current, &mut scratch);
    }
    Err(Error::NoAnswer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    const ID: u16 = 0x7942;
    const ADDRESS: [u8; 4] = [10, 0, 2, 2];
    fn response() -> Vec<u8> {
        let mut bytes = [0; MAX_BYTES]; let length = query(ID, &mut bytes).unwrap();
        let mut packet = bytes[..length].to_vec(); packet[2] = 0x81; packet[3] = 0x80; packet
    }
    fn append_a(packet: &mut Vec<u8>, compressed: bool, address: [u8; 4], ttl: u32) {
        let count = u16::from_be_bytes([packet[6], packet[7]]) + 1;
        packet[6..8].copy_from_slice(&count.to_be_bytes());
        packet.extend_from_slice(if compressed { &[0xC0, 0x0C] } else { NAME });
        packet.extend_from_slice(&[0, 1, 0, 1]); packet.extend_from_slice(&ttl.to_be_bytes());
        packet.extend_from_slice(&[0, 4]); packet.extend_from_slice(&address);
    }
    fn valid(compressed: bool) -> Vec<u8> {
        let mut packet = response(); append_a(&mut packet, compressed, ADDRESS, 60); packet
    }
    #[test] fn actual_query_bytes_and_nonzero_id() {
        let mut out = [0xA5; MAX_BYTES];
        assert_eq!(query(0, &mut out), Err(Error::InvalidId)); assert_eq!(out, [0xA5; MAX_BYTES]);
        assert_eq!(query(ID, &mut out), Ok(34));
        assert_eq!(&out[..34], b"\x79\x42\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x05probe\x05sexos\x04test\x00\x00\x01\x00\x01");
        assert!(out[34..].iter().all(|b| *b == 0xA5));
    }
    #[test] fn literal_and_exact_c00c_answers() {
        for compressed in [false, true] {
            assert_eq!(parse(ID, &valid(compressed)), Ok(Some(Answer { address: ADDRESS, ttl: 60 })));
        }
    }
    #[test] fn multiple_answers_return_first_after_complete_validation() {
        let mut packet = valid(false);
        for n in 1..8 { append_a(&mut packet, true, [192, 0, 2, n], u32::MAX); }
        assert_eq!(parse(ID, &packet), Ok(Some(Answer { address: ADDRESS, ttl: 0 })));
    }
    #[test] fn wrong_id_name_type_or_class_are_unrelated() {
        let mut packet = valid(true); assert_eq!(parse(ID+1, &packet), Ok(None));
        packet[13] = b'X'; assert_eq!(parse(ID, &packet), Ok(None));
        let mut packet = valid(true); packet[31] = 28; assert_eq!(parse(ID, &packet), Ok(None));
        let mut packet = valid(true); packet[33] = 3; assert_eq!(parse(ID, &packet), Ok(None));
    }
    #[test] fn every_truncation_cut_rejected_for_literal_and_compressed_answers() {
        for compressed in [false, true] {
            let packet = valid(compressed);
            for cut in 0..packet.len() { assert!(parse(ID, &packet[..cut]).is_err(), "cut={cut} compressed={compressed}"); }
        }
    }
    #[test] fn malformed_lengths_and_label_kinds_rejected() {
        let mut packet = valid(true); packet[12] = 63; assert_eq!(parse(ID, &packet), Err(Error::Truncated));
        for length in [0x40, 0x80] {
            let mut packet = valid(true); packet[12] = length; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        }
        for length in [0, 3, 5, 255] {
            let mut packet = valid(true); packet[45] = length; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        }
    }
    #[test] fn flags_rcode_and_reserved_opcode_rejected() {
        let mut packet = valid(true); packet[2] |= 2; assert_eq!(parse(ID, &packet), Err(Error::Truncated));
        for rcode in 1..16 {
            let mut packet = valid(true); packet[3] |= rcode; assert_eq!(parse(ID, &packet), Err(Error::Rcode(rcode)));
        }
        let mut packet = valid(true); packet[2] &= 0x7F; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        let mut packet = valid(true); packet[2] |= 8; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        let mut packet = valid(true); packet[3] |= 0x40; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
    }
    #[test] fn invalid_boundaries_loops_and_malformed_cname_rejected() {
        for pointer in [[0xC0, 0x22], [0xC0, 0], [0xC0, 0xFF], [0xFF, 0xFF]] {
            let mut packet = valid(true); packet[34..36].copy_from_slice(&pointer);
            assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        }
        let mut packet = valid(true); packet[12] = 0xC0; packet[13] = 12;
        assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        let mut packet = valid(true); packet[37] = 5; assert!(parse(ID, &packet).is_err());
        let mut packet = valid(false); packet[35] = b'X'; assert_eq!(parse(ID, &packet), Err(Error::NoAnswer));
    }
    #[test] fn counts_oversize_and_trailing_data_rejected() {
        for qd in [0, 2, 255] {
            let mut packet = valid(true); packet[5] = qd; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        }
        let mut packet = valid(true); packet[7] = 9; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        for field in [9, 11] {
            let mut packet = valid(true); packet[field] = 9; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        }
        let mut packet = valid(true); packet.resize(513, 0); assert_eq!(parse(ID, &packet), Err(Error::Oversized));
        let mut packet = valid(true); packet.push(0); assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        assert_eq!(parse(ID, &response()), Err(Error::NoAnswer));
    }
    #[test] fn malformed_later_answer_never_returns_partial_success() {
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0);
        packet[61] = 3; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0);
        packet[53] = 5; assert!(parse(ID, &packet).is_err());
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0); packet.pop();
        assert_eq!(parse(ID, &packet), Err(Error::Truncated));
    }
    #[test] fn zero_expected_or_response_id_rejected() {
        assert_eq!(parse(0, &valid(true)), Err(Error::InvalidId));
        let mut packet = valid(true); packet[..2].fill(0); assert_eq!(parse(ID, &packet), Err(Error::InvalidId));
    }
    fn wire(text: &str) -> Vec<u8> {
        let mut name = Vec::new();
        for label in text.split('.') { name.push(label.len() as u8); name.extend_from_slice(label.as_bytes()); }
        name.push(0); name
    }
    fn pointer(offset: usize) -> [u8; 2] { (0xC000 | offset as u16).to_be_bytes() }
    fn rr(packet: &mut Vec<u8>, section: usize, owner: &[u8], kind: u16, data: &[u8], ttl: u32) -> (usize, usize) {
        let field = 6 + section * 2;
        let count = u16::from_be_bytes([packet[field], packet[field+1]]) + 1;
        packet[field..field+2].copy_from_slice(&count.to_be_bytes());
        let start = packet.len(); packet.extend_from_slice(owner);
        packet.extend_from_slice(&kind.to_be_bytes()); packet.extend_from_slice(&1u16.to_be_bytes());
        packet.extend_from_slice(&ttl.to_be_bytes()); packet.extend_from_slice(&(data.len() as u16).to_be_bytes());
        let data_start = packet.len(); packet.extend_from_slice(data); (start, data_start)
    }
    fn chain(links: usize) -> Vec<u8> {
        let mut packet = response(); let mut owner = NAME.to_vec();
        for n in 0..links {
            let target = wire(&std::format!("alias{n}.sexos.test"));
            rr(&mut packet, 0, &owner, 5, &target, 50-n as u32); owner = target;
        }
        rr(&mut packet, 0, &owner, 1, &ADDRESS, 60); packet
    }
    #[test] fn casefold_question_owner_and_alias_target_without_changing_wire_query() {
        let mut packet = response();
        for byte in &mut packet[12..30] { *byte = byte.to_ascii_uppercase(); }
        let alias = wire("Alias.SEXos.Test");
        rr(&mut packet, 0, &wire("PROBE.sexos.TEST"), 5, &alias, 40);
        rr(&mut packet, 0, &wire("ALIAS.sexos.test"), 1, &ADDRESS, 30);
        assert_eq!(parse(ID, &packet), Ok(Some(Answer { address: ADDRESS, ttl: 30 })));
    }
    #[test] fn cname_chain_record_permutations_and_final_rrset_ttl_minimum() {
        for order in [[0,1,2,3],[3,2,1,0],[2,0,3,1],[1,3,0,2],[0,3,2,1],[2,1,0,3]] {
            let mut packet = response();
            for index in order {
                match index {
                    0 => { rr(&mut packet,0,NAME,5,&wire("a.sexos.test"),40); },
                    1 => { rr(&mut packet,0,&wire("a.sexos.test"),5,&wire("b.sexos.test"),30); },
                    2 => { rr(&mut packet,0,&wire("b.sexos.test"),1,&ADDRESS,50); },
                    _ => { rr(&mut packet,0,&wire("b.sexos.test"),1,&[192,0,2,9],20); },
                }
            }
            let address = if order.iter().position(|n| *n==2) < order.iter().position(|n| *n==3) { ADDRESS } else { [192,0,2,9] };
            assert_eq!(parse(ID,&packet),Ok(Some(Answer { address,ttl:20 })),"{order:?}");
        }
    }
    #[test] fn registered_label_suffix_and_prior_pointer_boundaries_are_supported() {
        let mut packet = response(); let mut target = b"\x05alias".to_vec(); target.extend_from_slice(&pointer(18));
        let (_, target_offset) = rr(&mut packet,0,&pointer(12),5,&target,25);
        let (owner_offset,_) = rr(&mut packet,0,&pointer(target_offset),1,&ADDRESS,40);
        rr(&mut packet,0,&pointer(owner_offset),1,&[192,0,2,1],15);
        assert_eq!(parse(ID,&packet),Ok(Some(Answer { address:ADDRESS,ttl:15 })));
    }
    #[test] fn compression_hop_limit_accepts16_and_rejects17_with_registered_boundaries() {
        for extras in [4,5] {
            let mut packet=response();let mut target=12;
            for _ in 0..8 {target=rr(&mut packet,0,&pointer(target),1,&ADDRESS,60).0;}
            for _ in 0..extras {let start=packet.len();let (owner,data)=rr(&mut packet,1,&pointer(target),2,&pointer(start),60);assert!(data>owner);target=data;}
            if extras==4 {assert_eq!(parse(ID,&packet),Ok(Some(Answer {address:ADDRESS,ttl:60})));}
            else {assert_eq!(parse(ID,&packet),Err(Error::Unsupported));}
        }
    }
    #[test] fn pointers_to_field_label_payload_self_forward_and_header_fail_closed() {
        for target in [0,4,13,31,34,35,36,40,46,255] {
            let mut packet=valid(true);packet[34..36].copy_from_slice(&pointer(target));
            assert!(parse(ID,&packet).is_err(),"target={target}");
        }
        let mut packet=valid(true);let data=46;
        rr(&mut packet,0,&pointer(data),1,&ADDRESS,60);
        assert_eq!(parse(ID,&packet),Err(Error::Unsupported));
    }
    #[test] fn expanded_name255_is_supported_but256_is_not() {
        for last in [61,62] {
            let mut target=Vec::new();
            for (length,byte) in [(63,b'a'),(63,b'b'),(63,b'c'),(last,b'd')] {target.push(length);target.extend(core::iter::repeat_n(byte,length as usize));}
            target.push(0);let mut packet=response();let (_,offset)=rr(&mut packet,0,&pointer(12),5,&target,20);
            rr(&mut packet,0,&pointer(offset),1,&ADDRESS,60);
            if last==61 {assert_eq!(target.len(),255);assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:20})));}
            else {assert_eq!(parse(ID,&packet),Err(Error::Malformed));}
        }
    }
    #[test] fn cname_four_links_are_accepted_fifth_and_cycles_are_rejected() {
        assert_eq!(parse(ID,&chain(4)),Ok(Some(Answer{address:ADDRESS,ttl:47})));
        assert_eq!(parse(ID,&chain(5)),Err(Error::Unsupported));
        let mut packet=response();rr(&mut packet,0,NAME,5,&pointer(12),60);
        assert_eq!(parse(ID,&packet),Err(Error::Malformed));
        let mut packet=response();rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),60);rr(&mut packet,0,&wire("alias.sexos.test"),5,NAME,60);
        assert_eq!(parse(ID,&packet),Err(Error::Malformed));
    }
    #[test] fn cname_conflicts_and_a_coexistence_in_later_records_prevent_success() {
        for different in [false,true] {
            let mut packet=response();rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),60);
            if different {rr(&mut packet,0,NAME,5,&wire("evil.sexos.test"),60);}
            else {rr(&mut packet,0,NAME,1,&ADDRESS,60);}
            rr(&mut packet,0,&wire("alias.sexos.test"),1,&ADDRESS,60);
            assert_eq!(parse(ID,&packet),Err(Error::Malformed));
        }
        let mut packet=valid(true);rr(&mut packet,2,NAME,5,&wire("elsewhere.sexos.test"),60);
        assert_eq!(parse(ID,&packet),Err(Error::Malformed));
        let mut packet=response();rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),60);rr(&mut packet,2,NAME,28,&[0;16],60);
        assert_eq!(parse(ID,&packet),Err(Error::Malformed));
        let mut packet=response();rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),60);rr(&mut packet,0,NAME,5,&wire("ALIAS.SEXOS.TEST"),10);rr(&mut packet,0,&wire("alias.sexos.test"),1,&ADDRESS,40);
        assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:10})));
    }
    #[test] fn unrelated_and_additional_a_cannot_supply_or_reduce_selected_answer() {
        let mut packet=response();rr(&mut packet,0,&wire("evil.sexos.test"),1,&ADDRESS,1);
        assert_eq!(parse(ID,&packet),Err(Error::NoAnswer));
        rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),60);rr(&mut packet,2,&wire("alias.sexos.test"),1,&ADDRESS,1);
        assert_eq!(parse(ID,&packet),Err(Error::NoAnswer));
        let mut packet=valid(true);rr(&mut packet,0,&wire("evil.sexos.test"),1,&[192,0,2,1],0);rr(&mut packet,2,NAME,1,&[192,0,2,2],0);
        assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:60})));
    }
    #[test] fn ttl_highbit_zero_and_minimum_across_chain_and_answer_rrset() {
        for high in [0x8000_0000,u32::MAX] {
            let mut packet=valid(true);rr(&mut packet,0,NAME,1,&[192,0,2,9],high);
            assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:0})));
            let mut packet=response();rr(&mut packet,0,NAME,5,&wire("alias.sexos.test"),high);rr(&mut packet,0,&wire("alias.sexos.test"),1,&ADDRESS,60);
            assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:0})));
        }
    }
    #[test] fn bounded_auxiliary_ns_soa_aaaa_and_a_validate_without_becoming_answers() {
        let mut packet=valid(true);let ns=wire("ns.sexos.test");rr(&mut packet,1,&pointer(18),2,&ns,60);
        let mut soa=ns.clone();soa.extend_from_slice(&wire("hostmaster.sexos.test"));soa.extend_from_slice(&[0;20]);rr(&mut packet,1,&pointer(18),6,&soa,60);
        rr(&mut packet,2,&ns,28,&[0;16],60);rr(&mut packet,2,&ns,1,&[192,0,2,1],60);
        assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:60})));
        for cut in 0..packet.len() {assert!(parse(ID,&packet[..cut]).is_err(),"cut={cut}");}
        let mut unsupported=valid(true);rr(&mut unsupported,2,&[0],41,&[],0);
        assert_eq!(parse(ID,&unsupported),Err(Error::Unsupported));
    }
    #[test] fn malformed_late_auxiliary_rdata_and_cname_extent_cannot_return_earlier_a() {
        for (kind,data) in [(2,&[0xC0][..]),(6,&[0,0,0][..]),(28,&[0;15][..]),(5,&[0,0][..])] {
            let mut packet=valid(true);rr(&mut packet,2,&wire("other.sexos.test"),kind,data,60);
            assert!(parse(ID,&packet).is_err(),"kind={kind}");
        }
    }
    #[test] fn sixteen_total_records_and_eight_extra_cap_are_explicit() {
        let mut packet=response();for _ in 0..8 {rr(&mut packet,0,&pointer(12),1,&ADDRESS,60);}
        for _ in 0..8 {rr(&mut packet,2,&pointer(12),1,&[192,0,2,1],0);}
        assert_eq!(parse(ID,&packet),Ok(Some(Answer{address:ADDRESS,ttl:60})));
        rr(&mut packet,2,&pointer(12),1,&ADDRESS,60);assert_eq!(parse(ID,&packet),Err(Error::Unsupported));
    }
    #[test] fn every_chain_truncation_and_late_invalid_label_preserve_whole_packet_validation() {
        let packet=chain(3);for cut in 0..packet.len() {assert!(parse(ID,&packet[..cut]).is_err(),"cut={cut}");}
        let mut packet=valid(true);rr(&mut packet,0,&[0x40],1,&ADDRESS,60);assert_eq!(parse(ID,&packet),Err(Error::Malformed));
    }

}
