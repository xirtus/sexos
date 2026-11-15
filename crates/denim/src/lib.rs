// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Denim: compression for memory pages, the codec of SexOS compressed RAM (J5).
//!
//! A cold 4 KiB page is packed tight, like denim's dense weave, and unpacked when it is
//! touched again. Three forms, smallest first:
//! - `Packed::Zero`: every byte is zero (no storage).
//! - `Packed::Fill(word)`: the page repeats one 8-byte word (no storage but the word).
//! - `Packed::Lz(n)`: an LZ4-style block of `n` bytes written into the caller's buffer.
//!
//! The block format is LZ4's: a token byte (literal length in the high nibble, match
//! length minus 4 in the low one, 15 meaning "more bytes follow"), the literals, then a
//! 2-byte little-endian offset. The last sequence has literals only. `no_std`, no
//! allocation: the hash table lives on the caller's stack.
#![no_std]

pub const PAGE: usize = 4096;
/// Worst case of an incompressible page: the caller's buffer must hold this much.
pub const MAX_PACKED: usize = PAGE + PAGE / 255 + 16;
const MIN_MATCH: usize = 4;
/// The last bytes of a block are always literals (as in LZ4), so matches never run past
/// the end and the decoder can copy without tail checks.
const LAST_LITERALS: usize = 5;
/// No match may start in the block's last 12 bytes.
const MFLIMIT: usize = 12;
const HASH_BITS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packed { Zero, Fill(u64), Lz(usize) }

fn word(page: &[u8; PAGE], at: usize) -> u64 { u64::from_le_bytes(page[at..at + 8].try_into().unwrap()) }

/// Packs `page`. For `Packed::Lz(n)` the block is `out[..n]`; when that would not save
/// at least an eighth of the page, the caller should keep the page as it is.
pub fn pack(page: &[u8; PAGE], out: &mut [u8; MAX_PACKED]) -> Packed {
    let first = word(page, 0);
    if (8..PAGE).step_by(8).all(|at| word(page, at) == first) {
        return if first == 0 { Packed::Zero } else { Packed::Fill(first) };
    }
    Packed::Lz(lz(page, out))
}

fn read4(b: &[u8], at: usize) -> u32 { u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) }
fn hash(v: u32) -> usize { (v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize }

fn put_length(out: &mut [u8], o: &mut usize, mut n: usize) {
    while n >= 255 { out[*o] = 255; *o += 1; n -= 255; }
    out[*o] = n as u8; *o += 1;
}

fn sequence(src: &[u8], out: &mut [u8], o: &mut usize, lit: core::ops::Range<usize>, matched: Option<(usize, usize)>) {
    let lits = lit.len();
    let ml = matched.map_or(0, |(_, len)| len - MIN_MATCH);
    let token = &mut out[*o]; *o += 1;
    *token = ((lits.min(15) as u8) << 4) | if matched.is_some() { ml.min(15) as u8 } else { 0 };
    if lits >= 15 { put_length(out, o, lits - 15); }
    out[*o..*o + lits].copy_from_slice(&src[lit]); *o += lits;
    if let Some((offset, _)) = matched {
        out[*o..*o + 2].copy_from_slice(&(offset as u16).to_le_bytes()); *o += 2;
        if ml >= 15 { put_length(out, o, ml - 15); }
    }
}

fn lz(src: &[u8; PAGE], out: &mut [u8; MAX_PACKED]) -> usize {
    let mut table = [0u16; 1 << HASH_BITS];
    let (mut i, mut anchor, mut o) = (0usize, 0usize, 0usize);
    // LZ4 blocks end with a match no closer than 12 bytes (MFLIMIT) to the end.
    let limit = PAGE - MFLIMIT;
    // Position 0 can never be a candidate (table holds 0 for "none"), which costs nothing.
    while i < limit {
        let v = read4(src, i);
        let h = hash(v);
        let candidate = table[h] as usize;
        table[h] = i as u16;
        if candidate == 0 || candidate >= i || i - candidate > u16::MAX as usize || read4(src, candidate) != v { i += 1; continue; }
        let mut len = MIN_MATCH;
        while i + len < PAGE - LAST_LITERALS && src[candidate + len] == src[i + len] { len += 1; }
        sequence(src, out, &mut o, anchor..i, Some((i - candidate, len)));
        i += len;
        anchor = i;
        if i >= 2 && i - 2 < limit { table[hash(read4(src, i - 2))] = (i - 2) as u16; }
    }
    sequence(src, out, &mut o, anchor..PAGE, None);
    o
}

/// Unpacks a block made by `pack` into `page`. `None` for a damaged block: wrong
/// lengths, an offset before the page, or a page that does not come out exactly full.
pub fn unpack(packed: Packed, block: &[u8], page: &mut [u8; PAGE]) -> Option<()> {
    match packed {
        Packed::Zero => { page.fill(0); Some(()) }
        Packed::Fill(w) => { for c in page.chunks_exact_mut(8) { c.copy_from_slice(&w.to_le_bytes()); } Some(()) }
        Packed::Lz(n) => unlz(block.get(..n)?, page),
    }
}

fn get_length(src: &[u8], i: &mut usize, mut n: usize) -> Option<usize> {
    loop {
        let b = *src.get(*i)?; *i += 1;
        n = n.checked_add(b as usize)?;
        if b != 255 { return Some(n); }
    }
}

fn unlz(src: &[u8], page: &mut [u8; PAGE]) -> Option<()> {
    let (mut i, mut o) = (0usize, 0usize);
    loop {
        let token = *src.get(i)?; i += 1;
        let mut lits = (token >> 4) as usize;
        if lits == 15 { lits = get_length(src, &mut i, lits)?; }
        let end = o.checked_add(lits)?;
        page.get_mut(o..end)?.copy_from_slice(src.get(i..i + lits)?);
        i += lits; o = end;
        if i == src.len() { return (o == PAGE).then_some(()); }
        let offset = u16::from_le_bytes([*src.get(i)?, *src.get(i + 1)?]) as usize; i += 2;
        if offset == 0 || offset > o { return None; }
        let mut len = (token & 15) as usize;
        if len == 15 { len = get_length(src, &mut i, len)?; }
        len += MIN_MATCH;
        if o + len > PAGE { return None; }
        // Byte by byte: a match may overlap the bytes it is copying (a run).
        for k in 0..len { page[o + k] = page[o + k - offset]; }
        o += len;
    }
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests {
    use super::*;
    fn round(page: &[u8; PAGE]) -> Packed {
        let mut out = [0u8; MAX_PACKED];
        let p = pack(page, &mut out);
        let mut back = [0xAAu8; PAGE];
        unpack(p, &out, &mut back).expect("unpack");
        assert!(back[..] == page[..]);
        p
    }
    /// Every match starts at least MFLIMIT bytes before the end and the block ends
    /// in at least LAST_LITERALS literals, as LZ4 decoders require.
    fn lz4_rules(block: &[u8]) {
        let (mut i, mut o) = (0usize, 0usize);
        let len = |i: &mut usize, mut n: usize| { loop { let b = block[*i]; *i += 1; n += b as usize; if b != 255 { return n; } } };
        loop {
            let token = block[i]; i += 1;
            let mut lits = (token >> 4) as usize;
            if lits == 15 { lits = len(&mut i, 15); }
            i += lits; o += lits;
            if i == block.len() { assert!(lits >= LAST_LITERALS && o == PAGE); return; }
            assert!(o + MFLIMIT <= PAGE, "match starts {} bytes before the end", PAGE - o);
            i += 2;
            let mut ml = (token & 15) as usize;
            if ml == 15 { ml = len(&mut i, 15); }
            o += ml + MIN_MATCH;
        }
    }
    #[test] fn last_match_keeps_lz4_end_rules() {
        // A short repeat right at the tail, after a long zero run (the review's case).
        let mut seed = 7u64;
        for at in 4070..PAGE - 4 {
            let mut page = [0u8; PAGE];
            for b in &mut page[4000..] { *b = rng(&mut seed) as u8; }
            let r = [1u8, 2, 3, 4];
            page[at - 40..at - 36].copy_from_slice(&r);
            page[at..at + 4].copy_from_slice(&r);
            let mut out = [0u8; MAX_PACKED];
            if let Packed::Lz(n) = pack(&page, &mut out) { lz4_rules(&out[..n]); }
            round(&page);
        }
    }
    fn rng(seed: &mut u64) -> u64 { *seed ^= *seed << 13; *seed ^= *seed >> 7; *seed ^= *seed << 17; *seed }
    #[test] fn zero_and_fill() {
        assert_eq!(round(&[0; PAGE]), Packed::Zero);
        let mut p = [0u8; PAGE]; for c in p.chunks_exact_mut(8) { c.copy_from_slice(&0x1122334455667788u64.to_le_bytes()); }
        assert_eq!(round(&p), Packed::Fill(0x1122334455667788));
    }
    #[test] fn text_shrinks() {
        let text = b"The quick brown fox jumps over the lazy dog. SexOS packs cold pages like denim. ";
        let mut p = [0u8; PAGE]; for (i, b) in p.iter_mut().enumerate() { *b = text[i % text.len()]; }
        match round(&p) { Packed::Lz(n) => assert!(n < 400, "{n}"), other => panic!("{other:?}") }
    }
    #[test] fn random_does_not_break() {
        let mut s = 0x9E3779B97F4A7C15u64;
        for _ in 0..200 {
            let mut p = [0u8; PAGE];
            for c in p.chunks_exact_mut(8) { c.copy_from_slice(&rng(&mut s).to_le_bytes()); }
            match round(&p) { Packed::Lz(n) => assert!(n <= MAX_PACKED), other => panic!("{other:?}") }
        }
    }
    #[test] fn mixed_pages_round_trip() {
        let mut s = 7u64;
        for k in 0..2000 {
            let mut p = [0u8; PAGE];
            // Sparse heap-like pages: runs of zeros, small integers, repeated structs.
            for i in 0..PAGE { let r = rng(&mut s); p[i] = match (r >> 60) as u8 { 0..=8 => 0, 9..=12 => (r & 7) as u8, _ => (i as u8).wrapping_mul(k as u8) }; }
            round(&p);
        }
    }
    #[test] fn damaged_blocks_are_refused() {
        let mut p = [0u8; PAGE]; for (i, b) in p.iter_mut().enumerate() { *b = (i % 61) as u8; }
        let mut out = [0u8; MAX_PACKED];
        let Packed::Lz(n) = pack(&p, &mut out) else { panic!() };
        let mut back = [0u8; PAGE];
        for cut in 0..n { let _ = unpack(Packed::Lz(cut), &out, &mut back); }
        let mut s = 3u64;
        for _ in 0..5000 { let mut bad = out; let at = (rng(&mut s) as usize) % n; bad[at] ^= 1 << (rng(&mut s) % 8); let _ = unpack(Packed::Lz(n), &bad, &mut back); }
    }
}
