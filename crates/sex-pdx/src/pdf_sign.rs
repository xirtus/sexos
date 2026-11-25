// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Vellum ↔ Shell ↔ Shibari signing route. Only public certificate bytes, a
//! bounded CMS attribute set, and the resulting DER signature cross this route.
//! The private scalar stays in the Shibari program's protection view.

pub const REGISTER: u64 = 0x6A0;
pub const LIST: u64 = 0x6A1;
pub const CERT: u64 = 0x6A2;
pub const POINT: u64 = 0x6A3;
pub const BEGIN: u64 = 0x6A4;
pub const CHUNK: u64 = 0x6A5;
pub const FINISH: u64 = 0x6A6;
pub const STATUS: u64 = 0x6A7;
pub const SIGNATURE: u64 = 0x6A8;
pub const END: u64 = 0x6A9;
pub const MAX_CERT: usize = 32 * 1024;
pub const POINT_BYTES: usize = 65;
pub const MAX_ATTRS: usize = 8192;
pub const MAX_SIGNATURE: usize = 80;
/// Status after the signer explicitly declined or closed.
pub const REFUSED: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signer { pub row: u32, pub certificate_bytes: usize }

pub fn pack(bytes: &[u8]) -> u64 {
    let mut word = [0u8; 8];
    word[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
    u64::from_le_bytes(word)
}
pub fn unpack(word: u64, into: &mut [u8]) {
    let n = into.len().min(8);
    into[..n].copy_from_slice(&word.to_le_bytes()[..n]);
}

#[cfg(target_arch = "x86_64")]
fn call(op: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    crate::pdx_call_checked(crate::SLOT_SHELL, op, a, b, c)
}

/// The single active signer whose certificate a person can select in Vellum.
#[cfg(target_arch = "x86_64")]
pub fn selected() -> Result<Option<Signer>, u64> {
    let value = call(LIST, 0, 0, 0)?;
    if value == 0 { return Ok(None); }
    let signer = Signer { row: (value >> 32) as u32, certificate_bytes: value as u32 as usize };
    if signer.row == 0 || signer.certificate_bytes == 0 || signer.certificate_bytes > MAX_CERT { return Err(REFUSED); }
    Ok(Some(signer))
}

#[cfg(target_arch = "x86_64")]
pub fn certificate(signer: Signer, into: &mut [u8]) -> Result<(), u64> {
    if into.len() != signer.certificate_bytes || into.len() > MAX_CERT { return Err(REFUSED); }
    for (i, chunk) in into.chunks_mut(8).enumerate() {
        unpack(call(CERT, signer.row as u64, i as u64, 0)?, chunk);
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
pub fn public_point(signer: Signer, into: &mut [u8; POINT_BYTES]) -> Result<(), u64> {
    for (i, chunk) in into.chunks_mut(8).enumerate() {
        unpack(call(POINT, signer.row as u64, i as u64, 0)?, chunk);
    }
    Ok(())
}

/// Queue one operation. The Shibari window asks its owner to approve it.
#[cfg(target_arch = "x86_64")]
pub fn request(signer: Signer, attrs: &[u8]) -> Result<(), u64> {
    if attrs.is_empty() || attrs.len() > MAX_ATTRS { return Err(REFUSED); }
    if call(BEGIN, signer.row as u64, attrs.len() as u64, 0)? != 0 { return Err(REFUSED); }
    for (i, chunk) in attrs.chunks(8).enumerate() {
        if call(CHUNK, signer.row as u64, i as u64, pack(chunk))? != 0 { let _ = call(END, signer.row as u64, 0, 0); return Err(REFUSED); }
    }
    if call(FINISH, signer.row as u64, 0, 0)? != 0 { let _ = call(END, signer.row as u64, 0, 0); return Err(REFUSED); }
    Ok(())
}

/// `None` while the Shibari person has not answered, or the DER bytes after approval.
#[cfg(target_arch = "x86_64")]
pub fn poll(signer: Signer, into: &mut [u8; MAX_SIGNATURE]) -> Result<Option<usize>, u64> {
    let len = call(STATUS, signer.row as u64, 0, 0)?;
    if len == 0 { return Ok(None); }
    if len == REFUSED || len > MAX_SIGNATURE as u64 { let _ = call(END, signer.row as u64, 0, 0); return Err(REFUSED); }
    for (i, chunk) in into[..len as usize].chunks_mut(8).enumerate() {
        unpack(call(SIGNATURE, signer.row as u64, i as u64, 0)?, chunk);
    }
    let _ = call(END, signer.row as u64, 0, 0);
    Ok(Some(len as usize))
}

/// Release a pending request after a timeout or a viewer cancellation.
#[cfg(target_arch = "x86_64")]
pub fn cancel(signer: Signer) { let _ = call(END, signer.row as u64, 0, 0); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_words_preserve_non_aligned_tail_without_exposing_padding() {
        let mut result = [0u8; 13];
        unpack(pack(b"12345678"), &mut result[..8]);
        unpack(pack(b"abcde"), &mut result[8..]);
        assert_eq!(&result, b"12345678abcde");
    }
}
