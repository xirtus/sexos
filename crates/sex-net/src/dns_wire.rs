// SPDX-License-Identifier: GPL-3.0-only
//! Narrow, allocation-free DNS A codec for the canonical `probe.sexos.test`.
//! This module carries protocol data only. It grants no endpoint or app authority.
//! Supported replies contain one exact uncompressed question, zero authority and
//! additional records, and one to eight direct IN A answers. Answer owner names
//! may be the exact literal name or the exact compression pointer C00C. CNAME,
//! other compression, EDNS, TCP fallback and caching are deliberately unsupported.
//! UDP source/destination endpoint binding belongs to the private transport job.

pub const MAX_BYTES: usize = 512;
pub const MAX_ANSWERS: usize = 8;
pub const QUERY_BYTES: usize = 34;
const NAME: &[u8] = b"\x05probe\x05sexos\x04test\x00";
const QUESTION: &[u8] = b"\x05probe\x05sexos\x04test\x00\x00\x01\x00\x01";

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
/// Consume one literal DNS name with an explicit DNS name-length bound. Names
/// are never followed through arbitrary pointers or copied into new storage.
fn literal_name<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], Error> {
    let start = *cursor;
    loop {
        let length = take(bytes, cursor, 1)?[0];
        match length & 0xC0 {
            0xC0 => return Err(Error::Unsupported),
            0x00 => {},
            _ => return Err(Error::Malformed),
        }
        if length != 0 { take(bytes, cursor, length as usize)?; }
        if *cursor - start > 255 { return Err(Error::Malformed); }
        if length == 0 { return Ok(&bytes[start..*cursor]); }
    }
}
fn answer_name(bytes: &[u8], cursor: &mut usize) -> Result<(), Error> {
    let first = *bytes.get(*cursor).ok_or(Error::Truncated)?;
    if first & 0xC0 == 0xC0 {
        if take(bytes, cursor, 2)? != [0xC0, 0x0C] { return Err(Error::Unsupported); }
    } else if literal_name(bytes, cursor)? != NAME {
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// Parse an endpoint-bound response. `None` means another transaction or a
/// different well-formed question; it never supplies a usable address. Matching
/// responses validate every supported answer and the complete datagram before
/// returning the first A address. TTL is returned as data without a cache claim.
pub fn parse(expected_id: u16, response: &[u8]) -> Result<Option<Answer>, Error> {
    if expected_id == 0 { return Err(Error::InvalidId); }
    if response.len() > MAX_BYTES { return Err(Error::Oversized); }
    if response.len() < 12 { return Err(Error::Truncated); }
    let id = u16_at(response, 0)?;
    if id == 0 { return Err(Error::InvalidId); }
    if id != expected_id { return Ok(None); }
    if u16_at(response, 4)? != 1 { return Err(Error::Malformed); }
    let mut cursor = 12;
    let name = literal_name(response, &mut cursor)?;
    let question_tail = take(response, &mut cursor, 4)?;
    if name != NAME || question_tail != [0, 1, 0, 1] { return Ok(None); }

    let flags = u16_at(response, 2)?;
    if flags & 0x8000 == 0 { return Err(Error::Malformed); }
    if flags & 0x7840 != 0 { return Err(Error::Unsupported); } // opcode and reserved Z.
    if flags & 0x0200 != 0 { return Err(Error::Truncated); }
    if flags & 0x000F != 0 { return Err(Error::Rcode((flags & 15) as u8)); }
    let count = u16_at(response, 6)? as usize;
    if count > MAX_ANSWERS { return Err(Error::Unsupported); }
    if u16_at(response, 8)? != 0 || u16_at(response, 10)? != 0 { return Err(Error::Unsupported); }

    let mut first_answer = None;
    for _ in 0..count {
        answer_name(response, &mut cursor)?;
        let fields = take(response, &mut cursor, 10)?;
        let record_type = u16::from_be_bytes([fields[0], fields[1]]);
        let class = u16::from_be_bytes([fields[2], fields[3]]);
        let ttl = u32::from_be_bytes([fields[4], fields[5], fields[6], fields[7]]);
        let length = u16::from_be_bytes([fields[8], fields[9]]) as usize;
        if record_type != 1 || class != 1 { return Err(Error::Unsupported); }
        if length != 4 { return Err(Error::Malformed); }
        let data = take(response, &mut cursor, length)?;
        if first_answer.is_none() {
            first_answer = Some(Answer { address: [data[0], data[1], data[2], data[3]], ttl });
        }
    }
    if cursor != response.len() { return Err(Error::Malformed); }
    first_answer.map(Some).ok_or(Error::NoAnswer)
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
        assert_eq!(parse(ID, &packet), Ok(Some(Answer { address: ADDRESS, ttl: 60 })));
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
    #[test] fn arbitrary_pointers_loops_and_cname_rejected() {
        for pointer in [[0xC0, 0x22], [0xC0, 0], [0xC0, 0xFF], [0xFF, 0xFF]] {
            let mut packet = valid(true); packet[34..36].copy_from_slice(&pointer);
            assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        }
        let mut packet = valid(true); packet[12] = 0xC0; packet[13] = 12;
        assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        let mut packet = valid(true); packet[37] = 5; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        let mut packet = valid(false); packet[35] = b'X'; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
    }
    #[test] fn counts_oversize_and_trailing_data_rejected() {
        for qd in [0, 2, 255] {
            let mut packet = valid(true); packet[5] = qd; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        }
        let mut packet = valid(true); packet[7] = 9; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        for field in [9, 11] {
            let mut packet = valid(true); packet[field] = 1; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        }
        let mut packet = valid(true); packet.resize(513, 0); assert_eq!(parse(ID, &packet), Err(Error::Oversized));
        let mut packet = valid(true); packet.push(0); assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        assert_eq!(parse(ID, &response()), Err(Error::NoAnswer));
    }
    #[test] fn malformed_later_answer_never_returns_partial_success() {
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0);
        packet[61] = 3; assert_eq!(parse(ID, &packet), Err(Error::Malformed));
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0);
        packet[53] = 5; assert_eq!(parse(ID, &packet), Err(Error::Unsupported));
        let mut packet = valid(true); append_a(&mut packet, true, [192, 0, 2, 1], 0); packet.pop();
        assert_eq!(parse(ID, &packet), Err(Error::Truncated));
    }
    #[test] fn zero_expected_or_response_id_rejected() {
        assert_eq!(parse(0, &valid(true)), Err(Error::InvalidId));
        let mut packet = valid(true); packet[..2].fill(0); assert_eq!(parse(ID, &packet), Err(Error::InvalidId));
    }
}
