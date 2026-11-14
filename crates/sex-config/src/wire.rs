// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The settings protocol between Files and its callers; see `sex_pdx::config`.

pub const OP: u64 = 0x2F0;

/// arg1 unused. Reply: the key's value.
pub const GET: u8 = 1;
/// arg1 = the new value (i32). Reply: the value, now held.
pub const SET: u8 = 2;
/// Reply: the generation (low 32 bits) in the value field.
pub const GENERATION: u8 = 3;
/// key = the last key seen (0 to start), arg1 = generation the watcher last saw.
/// Reply: the next changed key and its value, or status `DONE`.
pub const NEXT: u8 = 4;

pub const OK: u8 = 0;
pub const UNKNOWN_KEY: u8 = 1;
pub const OUT_OF_RANGE: u8 = 2;
pub const DENIED: u8 = 3;
/// The settings could not be written to disk; the value still holds until restart.
pub const NOT_SAVED: u8 = 4;
pub const DONE: u8 = 5;
pub const INVALID: u8 = 6;

const TAG: u64 = 0xC6;

pub const fn request(verb: u8, key: u16) -> u64 { verb as u64 | (key as u64) << 8 }
/// (verb, key), or None for a word with reserved bits set.
pub const fn decode_request(arg0: u64) -> Option<(u8, u16)> {
    if arg0 >> 24 != 0 { return None; }
    Some((arg0 as u8, (arg0 >> 8) as u16))
}

/// One answer: tag, status, key and value in one word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply { pub status: u8, pub key: u16, pub value: i32 }
impl Reply {
    pub const fn word(self) -> u64 {
        TAG << 56 | (self.value as u32 as u64) << 24 | (self.key as u64) << 8 | self.status as u64
    }
    pub const fn decode(word: u64) -> Option<Reply> {
        if word >> 56 != TAG { return None; }
        Some(Reply { status: word as u8, key: (word >> 8) as u16, value: (word >> 24) as u32 as i32 })
    }
}

/// Words for a status.
pub fn message(status: u8) -> &'static str {
    match status {
        OK => "done",
        UNKNOWN_KEY => "no such setting",
        OUT_OF_RANGE => "value out of range",
        DENIED => "only Settings may change settings",
        NOT_SAVED => "changed, but not saved to disk",
        DONE => "no more changes",
        _ => "bad request",
    }
}

/// Answers one request against `config`. `writer` says whether the kernel-named
/// caller may change settings; `save` writes the changed table to disk and says
/// whether it could. A SET that changes nothing saves nothing.
pub fn serve(config: &mut crate::Config, writer: bool, arg0: u64, arg1: u64, save: impl FnOnce(&crate::Config) -> bool) -> Reply {
    let Some((verb, key)) = decode_request(arg0) else { return Reply { status: INVALID, key: 0, value: 0 } };
    let answer = |status, value| Reply { status, key, value };
    match verb {
        GET => match config.get(key) { Some(v) => answer(OK, v), None => answer(UNKNOWN_KEY, 0) },
        SET => {
            if !writer { return answer(DENIED, 0); }
            let value = arg1 as u32 as i32;
            if arg1 >> 32 != 0 { return answer(OUT_OF_RANGE, 0); }
            match config.set(key, value) {
                Ok(true) => answer(if save(config) { OK } else { NOT_SAVED }, value),
                Ok(false) => answer(OK, value),
                Err(crate::SetError::UnknownKey) => answer(UNKNOWN_KEY, 0),
                Err(crate::SetError::OutOfRange) => answer(OUT_OF_RANGE, 0),
            }
        }
        GENERATION => answer(OK, config.generation() as u32 as i32),
        NEXT => match config.changed_since(arg1, key) {
            Some((k, v)) => Reply { status: OK, key: k, value: v },
            None => Reply { status: DONE, key: 0, value: 0 },
        },
        _ => answer(INVALID, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn words_round_trip() {
        for r in [Reply { status: OK, key: 7, value: -3 }, Reply { status: DONE, key: 0, value: 0x7FFF_FFFF }, Reply { status: 6, key: 0xFFFF, value: i32::MIN }] {
            assert_eq!(Reply::decode(r.word()), Some(r));
        }
        assert_eq!(Reply::decode(0xF5 << 56), None);
        assert_eq!(decode_request(request(SET, 300)), Some((SET, 300)));
        assert_eq!(decode_request(1 << 30), None);
    }
    #[test]
    fn serve_checks_writer_range_and_saves_once() {
        use crate::{id, Config};
        let mut c = Config::new();
        let mut saves = 0;
        let set = |k: u16, v: i64| (request(SET, k), v as u32 as u64);
        let (a, b) = set(id::WALLPAPER, 2);
        assert_eq!(serve(&mut c, false, a, b, |_| { saves += 1; true }).status, DENIED);
        assert_eq!(serve(&mut c, true, a, b, |_| { saves += 1; true }), Reply { status: OK, key: id::WALLPAPER, value: 2 });
        assert_eq!(serve(&mut c, true, a, b, |_| { saves += 1; true }).status, OK, "same value");
        assert_eq!(saves, 1);
        let (a, b) = set(id::NIGHT_LIGHT, -1);
        assert_eq!(serve(&mut c, true, a, b, |_| true).status, OUT_OF_RANGE);
        assert_eq!(serve(&mut c, true, request(SET, id::VOLUME), 1 << 32, |_| true).status, OUT_OF_RANGE);
        let (a, b) = set(id::VOLUME, 10);
        assert_eq!(serve(&mut c, true, a, b, |_| false).status, NOT_SAVED);
        assert_eq!(c.value(id::VOLUME), 10, "held even when not saved");
        assert_eq!(serve(&mut c, false, request(GET, id::WALLPAPER), 0, |_| true).value, 2);
        assert_eq!(serve(&mut c, false, request(GET, 999), 0, |_| true).status, UNKNOWN_KEY);
        assert_eq!(serve(&mut c, false, request(GENERATION, 0), 0, |_| true).value, 2);
        // A watcher that saw generation 1 gets each change after it.
        let r = serve(&mut c, false, request(NEXT, 0), 1, |_| true);
        assert_eq!((r.status, r.key, r.value), (OK, id::VOLUME, 10));
        assert_eq!(serve(&mut c, false, request(NEXT, r.key), 1, |_| true).status, DONE);
        assert_eq!(serve(&mut c, false, 7 << 30, 0, |_| true).status, INVALID);
    }
}
