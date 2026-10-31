// SPDX-License-Identifier: GPL-3.0-only
//! Pure BOOT-held q35 CMOS UTC source ABI. No I/O, clock writes, capabilities,
//! tick fallback, or confidence claim. The kernel supplies bounded locked pairs.
//! The named profile requires an explicit century and civil years 2020..=2099.
//! Register modes follow QEMU v11.0.2 include/hw/rtc/mc146818rtc_regs.h and
//! hw/rtc/mc146818rtc.c; this does not establish physical clock authenticity.

pub const SLOT: u64 = 61;
pub const SYS: u64 = 73;
pub const INFO: u64 = 1;
pub const READ: u64 = 2;
pub const CMOS_UTC: u32 = 1;
pub const ATTEMPTS: usize = 4;
pub const PRECISION_SECONDS: u64 = 1;
pub const DENIED: u64 = u64::MAX;
pub const UNAVAILABLE: u64 = u64::MAX - 1;
pub const BUSY: u64 = u64::MAX - 2;
pub const INVALID: u64 = u64::MAX - 3;
pub const MIN_SECONDS: u64 = 1_577_836_800; // 2020-01-01T00:00:00Z
pub const MAX_SECONDS: u64 = 4_102_444_799; // 2099-12-31T23:59:59Z

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub pd: u32, pub task: u32 }
/// The fixed BOOT role publishing civil-time snapshots to the status bar.
/// BOOT checks both components before granting this role its own source.
/// This tuple is protocol data; a runtime sender still needs its held route.
pub const STATUS_BAR: Identity = Identity { pd: 6, task: 6 };
impl Identity {
    pub const fn valid(self) -> bool { self.pd != 0 && self.task != 0 }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant { pub holder: Identity, pub consumer: Identity, pub source: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info { pub consumer: Identity, pub source: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot { pub seconds: u64, pub attempts: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raw {
    pub sec: u8, pub min: u8, pub hour: u8, pub day: u8, pub month: u8,
    pub year: u8, pub century: u8,
    pub status_a: u8, pub status_b: u8, pub status_d: u8,
}

fn result(raw: &[u64; 5]) -> Result<(), u64> {
    match raw[0] {
        0 => Ok(()),
        DENIED | UNAVAILABLE | BUSY | INVALID if raw[1..].iter().all(|v| *v == 0) => Err(raw[0]),
        _ => Err(INVALID),
    }
}
impl Info {
    /// Metadata is not authority. Reject every noncanonical output register.
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        result(&raw)?;
        let consumer = Identity { pd: (raw[2] >> 32) as u32, task: raw[2] as u32 };
        if raw[1] != CMOS_UTC as u64 || !consumer.valid()
            || raw[3] != ATTEMPTS as u64 || raw[4] != PRECISION_SECONDS { return Err(INVALID); }
        Ok(Self { consumer, source: CMOS_UTC })
    }
}
impl Snapshot {
    pub fn decode(raw: [u64; 5]) -> Result<Self, u64> {
        result(&raw)?;
        if !(MIN_SECONDS..=MAX_SECONDS).contains(&raw[1]) || raw[2] == 0
            || raw[2] > ATTEMPTS as u64 || raw[3] != PRECISION_SECONDS || raw[4] != 0 {
            return Err(INVALID);
        }
        Ok(Self { seconds: raw[1], attempts: raw[2] as usize })
    }
}
fn number(value: u8, binary: bool) -> Option<u32> {
    if binary { return Some(value as u32); }
    if value & 15 > 9 || value >> 4 > 9 { return None; }
    Some((value >> 4) as u32 * 10 + (value & 15) as u32)
}
fn leap(year: u32) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }
fn month_days(year: u32, month: u32) -> u32 {
    match month { 2 => 28 + leap(year) as u32, 4 | 6 | 9 | 11 => 30, 1 | 3 | 5 | 7 | 8 | 10 | 12 => 31, _ => 0 }
}
fn seconds(raw: Raw) -> Option<u64> {
    let binary = raw.status_b & 4 != 0;
    let sec = number(raw.sec, binary)?;
    let min = number(raw.min, binary)?;
    let mut hour = number(if raw.status_b & 2 == 0 { raw.hour & 0x7f } else { raw.hour }, binary)?;
    if raw.status_b & 2 == 0 {
        if !(1..=12).contains(&hour) { return None; }
        hour = hour % 12 + if raw.hour & 0x80 != 0 { 12 } else { 0 };
    }
    let day = number(raw.day, binary)?;
    let month = number(raw.month, binary)?;
    let yy = number(raw.year, binary)?;
    let century = number(raw.century, binary)?;
    if yy > 99 || century != 20 { return None; }
    let year = century * 100 + yy;
    if !(2020..=2099).contains(&year) || sec > 59 || min > 59 || hour > 23
        || day == 0 || day > month_days(year, month) { return None; }
    let mut days = 0u64;
    for y in 1970..year { days += 365 + leap(y) as u64; }
    for m in 1..month { days += month_days(year, m) as u64; }
    Some((days + day as u64 - 1) * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64)
}

/// Reject authorization/arguments before any provider call; INFO never samples.
/// Each READ tries at most four pairs. A valid coherent sample wins immediately;
/// coherent invalid calendar/mode data returns INVALID immediately. Missing pairs
/// or a coherent invalid-VRT source may retry; UIP/SET or unequal pairs may retry.
/// On exhaustion, any incoherence yields BUSY, otherwise UNAVAILABLE. All errors
/// clear all four data outputs; no old snapshot or predicted time is returned.
pub fn invoke(
    grant: Option<&Grant>, caller: Identity, kind: u64, reserved: [u64; 3],
    mut pairprovider: impl FnMut() -> Option<(Raw, Raw)>,
) -> [u64; 5] {
    let Some(grant) = grant else { return [DENIED, 0, 0, 0, 0]; };
    if !caller.valid() || !grant.holder.valid() || !grant.consumer.valid()
        || caller != grant.holder || grant.source != CMOS_UTC
        || !matches!(kind, INFO | READ) || reserved != [0; 3] { return [DENIED, 0, 0, 0, 0]; }
    if kind == INFO {
        return [0, CMOS_UTC as u64, ((grant.consumer.pd as u64) << 32) | grant.consumer.task as u64,
            ATTEMPTS as u64, PRECISION_SECONDS];
    }
    let mut busy = false;
    for attempt in 1..=ATTEMPTS {
        let Some((a, b)) = pairprovider() else { continue; };
        if a != b || a.status_a & 0x80 != 0 || a.status_b & 0x80 != 0 { busy = true; continue; }
        // The sanctioned q35 UTC profile requires the running32kHz divider
        // and no daylight-saving interpretation; stopped or local/DST clocks
        // cannot silently become certificate time.
        if a.status_d & 0x80 == 0 || a.status_a & 0x70 != 0x20 { continue; }
        if a.status_b & 1 != 0 { return [INVALID,0,0,0,0]; }
        let Some(value) = seconds(a) else { return [INVALID, 0, 0, 0, 0]; };
        return [0, value, attempt as u64, PRECISION_SECONDS, 0];
    }
    [if busy { BUSY } else { UNAVAILABLE }, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOLDER: Identity = Identity { pd: 13, task: 13 };
    const CONSUMER: Identity = Identity { pd: 15, task: 15 };
    const GRANT: Grant = Grant { holder: HOLDER, consumer: CONSUMER, source: CMOS_UTC };
    const RAW: Raw = Raw { sec: 0, min: 0, hour: 0, day: 1, month: 1, year: 0x20, century: 0x20,
        status_a: 0x26, status_b: 2, status_d: 0x80 };
    fn read(raw: Raw) -> [u64; 5] { invoke(Some(&GRANT), HOLDER, READ, [0; 3], || Some((raw, raw))) }
    fn forbidden() -> Option<(Raw, Raw)> { panic!("rejected request reached provider") }
    #[test]
    fn exact_authority_before_provider() {
        for caller in [Identity {pd:0,task:13}, Identity {pd:13,task:0}, CONSUMER, Identity {pd:13,task:14}] {
            assert_eq!(invoke(Some(&GRANT),caller,READ,[0;3],forbidden),[DENIED,0,0,0,0]);
        }
        assert_eq!(invoke(None,HOLDER,READ,[0;3],forbidden),[DENIED,0,0,0,0]);
        for grant in [Grant {source:0,..GRANT},Grant {source:2,..GRANT},Grant {holder:CONSUMER,..GRANT},Grant {consumer:Identity {pd:0,task:15},..GRANT},Grant {consumer:Identity {pd:15,task:0},..GRANT}] {
            assert_eq!(invoke(Some(&grant),HOLDER,READ,[0;3],forbidden),[DENIED,0,0,0,0]);
        }
    }
    #[test]
    fn full_arguments_and_info_no_provider() {
        for kind in [0,3,INFO+(1<<32),READ+(1<<32),u64::MAX] {
            assert_eq!(invoke(Some(&GRANT),HOLDER,kind,[0;3],forbidden),[DENIED,0,0,0,0]);
        }
        for at in 0..3 { let mut reserved=[0;3];reserved[at]=1<<63;
            assert_eq!(invoke(Some(&GRANT),HOLDER,READ,reserved,forbidden),[DENIED,0,0,0,0]); }
        let raw=invoke(Some(&GRANT),HOLDER,INFO,[0;3],forbidden);
        assert_eq!(raw,[0,1,(15<<32)|15,4,1]);
        assert_eq!(Info::decode(raw),Ok(Info {consumer:CONSUMER,source:CMOS_UTC}));
    }
    #[test]
    fn absent_busy_and_vrt_have_exact_bound_and_zero_outputs() {
        for case in 0..4 { let mut calls=0;
            let raw=invoke(Some(&GRANT),HOLDER,READ,[0;3],||{calls+=1;match case {
                0=>None,1=>Some((RAW,Raw {sec:1,..RAW})),2=>Some((Raw {status_a:0x80,..RAW},Raw {status_a:0x80,..RAW})),
                _=>Some((Raw {status_d:0,..RAW},Raw {status_d:0,..RAW})) }});
            assert_eq!(calls,4);assert_eq!(raw,[if case==1||case==2 {BUSY}else{UNAVAILABLE},0,0,0,0]);
        }
        let set=Raw {status_b:0x82,..RAW};assert_eq!(read(set),[BUSY,0,0,0,0]);
    }
    #[test]
    fn retries_precedence_and_actual_attempt_receipt() {
        for wanted in 1..=4 {let mut calls=0;
            let raw=invoke(Some(&GRANT),HOLDER,READ,[0;3],||{calls+=1;if calls==wanted {Some((RAW,RAW))}else{None}});
            assert_eq!(raw,[0,MIN_SECONDS,wanted,1,0]);assert_eq!(calls,wanted);
        }
        let mut calls=0;let raw=invoke(Some(&GRANT),HOLDER,READ,[0;3],||{calls+=1;if calls==1 {Some((RAW,Raw {min:1,..RAW}))}else{None}});
        assert_eq!(raw,[BUSY,0,0,0,0]);
        let mut calls=0;let invalid=Raw {month:0,..RAW};
        assert_eq!(invoke(Some(&GRANT),HOLDER,READ,[0;3],||{calls+=1;Some((invalid,invalid))}),[INVALID,0,0,0,0]);assert_eq!(calls,1);
    }
    #[test]
    fn bcd_binary_24h_and_epoch_boundaries() {
        assert_eq!(read(RAW),[0,MIN_SECONDS,1,1,0]);
        assert_eq!(read(Raw {year:20,century:20,status_b:6,..RAW}),[0,MIN_SECONDS,1,1,0]);
        assert_eq!(read(Raw {sec:0x59,min:0x59,hour:0x23,day:0x31,month:0x12,year:0x99,..RAW}),[0,MAX_SECONDS,1,1,0]);
        assert_eq!(read(Raw {sec:59,min:59,hour:23,day:31,month:12,year:99,century:20,status_b:6,..RAW}),[0,MAX_SECONDS,1,1,0]);
    }
    #[test]
    fn twelve_hour_am_pm_bcd_and_binary() {
        for binary in [false,true] {let base=Raw {status_b:if binary{4}else{0},year:if binary{20}else{0x20},century:if binary{20}else{0x20},..RAW};
            for (hour,delta) in [(if binary{12}else{0x12},0),(if binary{12}else{0x12}|0x80,43200),(1,3600),(0x81,46800),(if binary{11}else{0x11}|0x80,82800)] {
                assert_eq!(read(Raw {hour,..base}),[0,MIN_SECONDS+delta,1,1,0]);
            }
            for hour in [0,13,0x80,0x8d] {assert_eq!(read(Raw {hour,..base}),[INVALID,0,0,0,0]);}
        }
    }
    #[test]
    fn gregorian_leap_and_calendar_invalidity() {
        assert_eq!(read(Raw {month:2,day:0x29,..RAW})[1],1_582_934_400);
        for raw in [Raw {year:0x21,month:2,day:0x29,..RAW},Raw {month:4,day:0x31,..RAW},Raw {day:0,..RAW},Raw {month:0,..RAW},Raw {month:0x13,..RAW},Raw {sec:0x60,..RAW},Raw {min:0x60,..RAW},Raw {hour:0x24,..RAW},Raw {hour:0x80,..RAW},Raw {year:0x19,..RAW},Raw {century:0,..RAW},Raw {century:0x19,..RAW},Raw {century:0x21,..RAW}] {
            assert_eq!(read(raw),[INVALID,0,0,0,0]);
        }
        assert!(leap(2000));assert!(!leap(2100));assert!(!leap(1900));
    }
    #[test]
    fn malformed_bcd_and_binary_ranges_rejected() {
        for at in 0..7 {let mut raw=RAW;match at {0=>raw.sec=0x0a,1=>raw.min=0xa0,2=>raw.hour=0x1a,3=>raw.day=0x1f,4=>raw.month=0x0c,5=>raw.year=0x2a,_=>raw.century=0x2a};assert_eq!(read(raw),[INVALID,0,0,0,0]);}
        for raw in [Raw {year:100,century:20,status_b:6,..RAW},Raw {hour:24,year:20,century:20,status_b:6,..RAW},Raw {sec:60,year:20,century:20,status_b:6,..RAW}] {assert_eq!(read(raw),[INVALID,0,0,0,0]);}
    }
    #[test]
    fn complete_raw_pair_includes_mode_century_and_status() {
        for at in 0..10 {let mut b=RAW;match at {0=>b.sec^=1,1=>b.min^=1,2=>b.hour^=1,3=>b.day^=1,4=>b.month^=1,5=>b.year^=1,6=>b.century^=1,7=>b.status_a^=1,8=>b.status_b^=1,_=>b.status_d^=1};
            assert_eq!(invoke(Some(&GRANT),HOLDER,READ,[0;3],||Some((RAW,b))),[BUSY,0,0,0,0]);}
    }
    #[test]
    fn stopped_oscillator_or_daylight_saving_is_not_utc_time(){
        for status_a in [0,0x10,0x36,0x66,0x76]{assert_eq!(read(Raw{status_a,..RAW}),[UNAVAILABLE,0,0,0,0]);}
        assert_eq!(read(Raw{status_b:3,..RAW}),[INVALID,0,0,0,0]);
    }
    #[test]
    fn info_and_snapshot_decoders_validate_every_word() {
        let good=[0,1,(15<<32)|15,4,1];
        for (at,value) in [(0,1),(1,0),(1,1+(1<<32)),(2,15),(2,15<<32),(3,0),(3,5),(4,0),(4,2)] {let mut raw=good;raw[at]=value;assert_eq!(Info::decode(raw),Err(INVALID));}
        let good=[0,MIN_SECONDS,1,1,0];assert_eq!(Snapshot::decode(good),Ok(Snapshot {seconds:MIN_SECONDS,attempts:1}));
        for (at,value) in [(0,1),(1,MIN_SECONDS-1),(1,MAX_SECONDS+1),(2,0),(2,5),(2,1<<32),(3,0),(3,2),(4,1)] {let mut raw=good;raw[at]=value;assert_eq!(Snapshot::decode(raw),Err(INVALID));}
        for error in [DENIED,UNAVAILABLE,BUSY,INVALID] {
            assert_eq!(Info::decode([error,0,0,0,0]),Err(error));assert_eq!(Snapshot::decode([error,0,0,0,0]),Err(error));
            for at in 1..5 {let mut raw=[error,0,0,0,0];raw[at]=1;assert_eq!(Info::decode(raw),Err(INVALID));assert_eq!(Snapshot::decode(raw),Err(INVALID));}
        }
    }
}
