// SPDX-License-Identifier: GPL-3.0-only
//! BOOT-held CPU seed-source ABI and pure authorization/provider mechanism.
//! This module executes no CPU instruction and creates no capabilities. The
//! kernel supplies feature detection and one carry-checked sample operation;
//! health policy and private seed custody belong to the holder's service.

pub const SLOT: u64 = 60;
pub const SYS: u64 = 72;
pub const INFO: u64 = 1;
pub const READ: u64 = 2;
pub const RDSEED: u32 = 1;
pub const ATTEMPTS: usize = 8;
pub const DENIED: u64 = u64::MAX;
pub const UNSUPPORTED: u64 = u64::MAX - 1;
pub const BUSY: u64 = u64::MAX - 2;
pub const IO: u64 = u64::MAX - 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub pd: u32, pub task: u32 }
impl Identity {
    pub const fn valid(self) -> bool { self.pd != 0 && self.task != 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant { pub holder: Identity, pub consumer: Identity, pub source: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info { pub consumer: Identity, pub available: bool }

fn result(raw: &[u64; 5]) -> Result<(), u64> {
    match raw[0] {
        0 => Ok(()),
        DENIED | UNSUPPORTED | BUSY | IO if raw[1..].iter().all(|word| *word == 0) => Err(raw[0]),
        _ => Err(IO),
    }
}

impl Info {
    /// Validate the complete register result; malformed failure payloads never
    /// become data. Returned identity/configuration data grants no authority.
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        result(&raw)?;
        let consumer = Identity { pd: (raw[2] >> 32) as u32, task: raw[2] as u32 };
        if raw[1] != RDSEED as u64 || !consumer.valid()
            || raw[3] != ATTEMPTS as u64 || raw[4] > 1 { return Err(IO); }
        Ok(Self { consumer, available: raw[4] == 1 })
    }
}

/// Secret sample data: intentionally has no Debug implementation.
pub struct Word { pub value: u64, pub attempts: usize }
impl Word {
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        result(&raw)?;
        if raw[2] == 0 || raw[2] > ATTEMPTS as u64 || raw[3] != 0 || raw[4] != 0 {
            return Err(IO);
        }
        Ok(Self { value: raw[1], attempts: raw[2] as usize })
    }
}

/// The grant must come from the current domain's exact typed BOOT capability.
/// All authorization and argument checks precede either provider. None means
/// carry clear; Some(0) is a valid zero-valued sample, never a failure sentinel.
pub fn invoke(
    grant: Option<&Grant>, caller: Identity, kind: u64, reserved: [u64; 3],
    available: impl FnOnce() -> bool, mut sample: impl FnMut() -> Option<u64>,
) -> [u64; 5] {
    let denied = [DENIED, 0, 0, 0, 0];
    let Some(grant) = grant else { return denied; };
    if !caller.valid() || !grant.holder.valid() || !grant.consumer.valid()
        || grant.holder != caller || grant.source != RDSEED
        || !matches!(kind, INFO | READ) || reserved != [0; 3] { return denied; }
    let supported = available();
    if kind == INFO {
        return [0, RDSEED as u64,
            ((grant.consumer.pd as u64) << 32) | grant.consumer.task as u64,
            ATTEMPTS as u64, supported as u64];
    }
    if !supported { return [UNSUPPORTED, 0, 0, 0, 0]; }
    for attempt in 1..=ATTEMPTS {
        if let Some(word) = sample() { return [0, word, attempt as u64, 0, 0]; }
    }
    [BUSY, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    const HOLDER: Identity = Identity { pd: 13, task: 13 };
    const CONSUMER: Identity = Identity { pd: 15, task: 15 };
    const GRANT: Grant = Grant { holder: HOLDER, consumer: CONSUMER, source: RDSEED };
    fn no_available() -> bool { panic!("rejected request reached feature provider") }
    fn no_sample() -> Option<u64> { panic!("request reached forbidden sample provider") }

    #[test]
    fn absent_foreign_wrong_task_zero_and_bad_source_deny_before_providers() {
        for caller in [Identity {pd:0,task:13},Identity {pd:13,task:0},CONSUMER,Identity {pd:13,task:99}] {
            assert_eq!(invoke(Some(&GRANT),caller,READ,[0;3],no_available,no_sample),[DENIED,0,0,0,0]);
        }
        assert_eq!(invoke(None,HOLDER,READ,[0;3],no_available,no_sample),[DENIED,0,0,0,0]);
        for grant in [Grant {source:0,..GRANT},Grant {source:2,..GRANT},Grant {holder:Identity {pd:0,task:13},..GRANT},Grant {consumer:Identity {pd:15,task:0},..GRANT},Grant {consumer:Identity {pd:0,task:15},..GRANT}] {
            assert_eq!(invoke(Some(&grant),HOLDER,READ,[0;3],no_available,no_sample),[DENIED,0,0,0,0]);
        }
    }
    #[test]
    fn full_width_kind_and_every_reserved_word_deny_before_providers() {
        for kind in [0,3,INFO+(1<<32),READ+(1<<32),u64::MAX] {
            assert_eq!(invoke(Some(&GRANT),HOLDER,kind,[0;3],no_available,no_sample),[DENIED,0,0,0,0]);
        }
        for at in 0..3 {
            let mut reserved=[0;3];reserved[at]=1<<63;
            assert_eq!(invoke(Some(&GRANT),HOLDER,READ,reserved,no_available,no_sample),[DENIED,0,0,0,0]);
        }
    }
    #[test]
    fn info_is_exact_supported_or_unavailable_metadata_without_sampling() {
        for available in [false,true] {
            let calls=Cell::new(0);
            let raw=invoke(Some(&GRANT),HOLDER,INFO,[0;3],||{calls.set(calls.get()+1);available},no_sample);
            assert_eq!(calls.get(),1);assert_eq!(raw,[0,1,(15<<32)|15,8,available as u64]);
            assert_eq!(Info::decode(raw),Ok(Info {consumer:CONSUMER,available}));
        }
    }
    #[test]
    fn unsupported_never_samples_and_carry_clear_stops_at_exact_eight_attempts() {
        assert_eq!(invoke(Some(&GRANT),HOLDER,READ,[0;3],||false,no_sample),[UNSUPPORTED,0,0,0,0]);
        let mut calls=0;
        assert_eq!(invoke(Some(&GRANT),HOLDER,READ,[0;3],||true,||{calls+=1;None}),[BUSY,0,0,0,0]);
        assert_eq!(calls,ATTEMPTS);
    }
    #[test]
    fn success_stops_at_exact_attempt_and_zero_sample_is_valid_carry_data() {
        for wanted in 1..=ATTEMPTS {
            let mut calls=0;
            let raw=invoke(Some(&GRANT),HOLDER,READ,[0;3],||true,||{calls+=1;if calls==wanted{Some(0)}else{None}});
            assert_eq!(raw,[0,0,wanted as u64,0,0]);assert_eq!(calls,wanted);
            let word=Word::decode(raw).unwrap();assert_eq!(word.value,0);assert_eq!(word.attempts,wanted);
        }
        assert_eq!(Word::decode([0,u64::MAX,1,0,0]).unwrap().value,u64::MAX);
    }
    #[test]
    fn info_decoder_rejects_all_noncanonical_words_and_dirty_errors() {
        let good=[0,1,(15<<32)|15,8,1];
        for(at,value)in [(0,1),(1,0),(1,1+(1<<32)),(2,15),(2,15<<32),(3,0),(3,9),(4,2),(4,1<<32)] {
            let mut raw=good;raw[at]=value;assert_eq!(Info::decode(raw),Err(IO));
        }
        for error in [DENIED,UNSUPPORTED,BUSY,IO] {
            assert_eq!(Info::decode([error,0,0,0,0]),Err(error));
            for at in 1..5 {let mut raw=[error,0,0,0,0];raw[at]=1;assert_eq!(Info::decode(raw),Err(IO));}
        }
    }
    #[test]
    fn word_decoder_checks_attempt_bounds_reserved_outputs_and_error_shape() {
        for raw in [[0,1,0,0,0],[0,1,9,0,0],[0,1,1,1,0],[0,1,1,0,1],[0,1,1<<32,0,0],[123,0,0,0,0]] {
            assert!(matches!(Word::decode(raw),Err(IO)));
        }
        for error in [DENIED,UNSUPPORTED,BUSY,IO] {
            assert!(matches!(Word::decode([error,0,0,0,0]),Err(value)if value==error));
            assert!(matches!(Word::decode([error,1,0,0,0]),Err(IO)));
        }
    }
}
