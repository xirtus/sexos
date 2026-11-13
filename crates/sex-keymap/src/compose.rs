// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Dead-key accents and Compose sequences, from the generated tables in
//! `compose_data.rs` (scripts/gen_keymap_compose.py).
use crate::compose_data::{ACCENTED, SEQUENCES};

/// The letter `base` with the combining `mark`, if Unicode has it precomposed.
pub fn accent(mark: char, base: char) -> Option<char> {
    let key = (mark as u32, base as u32);
    ACCENTED.binary_search_by(|e| (e.0, e.1).cmp(&key)).ok().and_then(|i| char::from_u32(ACCENTED[i].2))
}

/// What a dead key types on its own (followed by Space, or pressed twice).
pub fn spacing(mark: char) -> char {
    match mark as u32 {
        0x300 => '`', 0x301 => '´', 0x302 => '^', 0x303 => '~', 0x304 => '¯', 0x306 => '˘',
        0x307 => '˙', 0x308 => '¨', 0x30A => '˚', 0x30B => '˝', 0x30C => 'ˇ', 0x323 => '.',
        0x327 => '¸', 0x328 => '˛',
        _ => mark,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seq { Done(char), Prefix, Unknown }

/// Looks up a Compose sequence typed so far (one to three keys).
pub fn sequence(keys: &[char]) -> Seq {
    if keys.is_empty() || keys.len() > 3 { return Seq::Unknown; }
    let mut padded = [0u32; 3];
    for (p, k) in padded.iter_mut().zip(keys) { *p = *k as u32; }
    match SEQUENCES.binary_search_by(|e| e.0.cmp(&padded)) {
        Ok(i) => char::from_u32(SEQUENCES[i].1).map_or(Seq::Unknown, Seq::Done),
        // The first entry after the padded keys starts with them if any does.
        Err(i) => match SEQUENCES.get(i) {
            Some(e) if e.0[..keys.len()] == padded[..keys.len()] => Seq::Prefix,
            _ => Seq::Unknown,
        },
    }
}
