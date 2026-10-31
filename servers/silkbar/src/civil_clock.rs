// SPDX-License-Identifier: GPL-3.0-only
//! One bounded source read and one retained publication. Cadence only schedules
//! queries; it never supplies or advances civil time.
use sex_pdx::clock_source::{self as source, Info, Snapshot};
use silkbar_model::civil_clock::{State, Update};

pub struct Probe {
    next: u32,
    pending: Option<Update>,
    high_water: Option<u32>,
    rollback: bool,
    ended: bool,
}

impl Probe {
    pub const fn new() -> Self {
        Self { next: 1, pending: None, high_water: None, rollback: false, ended: false }
    }

    pub fn pending(&self) -> Option<Update> { self.pending }

    /// Reserve the sole empty outbox before querying. Full retains the exact
    /// sequence/value; retries never reread the source or change a publication.
    pub fn stage(
        &mut self,
        info: impl FnOnce() -> Result<Info, u64>,
        read: impl FnOnce() -> Result<Snapshot, u64>,
    ) -> bool {
        if self.pending.is_some() || self.ended { return false; }
        let mut update = Update { state: State::Unknown, seconds: 0, sequence: self.next };
        if self.next == u32::MAX {
            update.state = State::Exhausted;
        } else if self.rollback {
            update.state = State::Rollback;
        } else if info().is_ok_and(|i| i.consumer == source::STATUS_BAR && i.source == source::CMOS_UTC) {
            update.state = State::Unavailable;
            if let Ok(sample) = read() {
                if (source::MIN_SECONDS..=source::MAX_SECONDS).contains(&sample.seconds)
                    && (1..=source::ATTEMPTS).contains(&sample.attempts)
                {
                    let seconds = sample.seconds as u32;
                    if self.high_water.is_some_and(|high| seconds < high) {
                        self.rollback = true;
                        update.state = State::Rollback;
                    } else {
                        self.high_water = Some(seconds);
                        update.state = State::Snapshot;
                        update.seconds = seconds;
                    }
                }
            }
        }
        self.pending = Some(update);
        true
    }

    /// Call only after the ordinary held Display route accepts this exact item.
    pub fn accepted(&mut self) {
        if let Some(update) = self.pending.take() {
            if update.sequence == u32::MAX { self.ended = true; }
            else { self.next = update.sequence + 1; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: u64 = source::MIN_SECONDS + 123;
    fn info() -> Result<Info,u64> { Ok(Info { consumer: source::STATUS_BAR, source: source::CMOS_UTC }) }
    fn sample(value: u64) -> Result<Snapshot,u64> { Ok(Snapshot { seconds: value, attempts: 1 }) }
    fn forbidden_info() -> Result<Info,u64> { panic!("source metadata queried with occupied outbox") }
    fn forbidden_read() -> Result<Snapshot,u64> { panic!("unauthorized or unnecessary source read") }

    #[test]
    fn exact_metadata_precedes_sampling() {
        for metadata in [Err(source::DENIED), Ok(Info { consumer: source::Identity {pd:15,task:15}, source: source::CMOS_UTC }),
            Ok(Info { consumer: source::STATUS_BAR, source: 2 })] {
            let mut p=Probe::new(); assert!(p.stage(|| metadata,forbidden_read));
            let u=p.pending().unwrap();assert_eq!(u.state,State::Unknown);assert_eq!(u.seconds,0);
        }
    }

    #[test]
    fn full_retains_immutable_publication_without_any_source_work() {
        let mut p=Probe::new();assert!(p.stage(info,||sample(T)));let retained=p.pending();
        for _ in 0..1000 {assert!(!p.stage(forbidden_info,forbidden_read));assert_eq!(p.pending(),retained);}
        p.accepted();assert!(p.stage(info,||sample(T+1)));let u=p.pending().unwrap();
        assert_eq!(u.sequence,2);assert_eq!(u.seconds,(T+1)as u32);
    }

    #[test]
    fn errors_and_invalid_structs_clear_values_without_last_good_fallback() {
        for bad in [Err(source::BUSY),Err(source::UNAVAILABLE),Err(source::INVALID),Err(source::DENIED),
            sample(source::MIN_SECONDS-1),sample(source::MAX_SECONDS+1),
            Ok(Snapshot {seconds:T,attempts:0}),Ok(Snapshot {seconds:T,attempts:source::ATTEMPTS+1})] {
            let mut p=Probe::new();p.stage(info,||sample(T));p.accepted();p.stage(info,||bad);
            let u=p.pending().unwrap();assert_eq!(u.state,State::Unavailable);assert_eq!(u.seconds,0);
            assert_eq!(p.high_water,Some(T as u32));
        }
    }

    #[test]
    fn equal_seconds_are_distinct_real_queries_and_never_interpolated() {
        let mut p=Probe::new();
        for seq in 1..100 {p.stage(info,||sample(T));let u=p.pending().unwrap();assert_eq!(u.sequence,seq);assert_eq!(u.seconds,T as u32);p.accepted();}
    }

    #[test]
    fn rollback_survives_unknown_or_failed_source_and_stops_source_queries() {
        let mut p=Probe::new();p.stage(info,||sample(T));p.accepted();
        p.stage(||Err(source::DENIED),forbidden_read);assert_eq!(p.pending().unwrap().state,State::Unknown);p.accepted();
        p.stage(info,||sample(T-1));assert_eq!(p.pending().unwrap().state,State::Rollback);assert_eq!(p.pending().unwrap().seconds,0);p.accepted();
        for _ in 0..10 {p.stage(forbidden_info,forbidden_read);assert_eq!(p.pending().unwrap().state,State::Rollback);p.accepted();}
    }

    #[test]
    fn exhaustion_reserves_terminal_value_and_never_wraps() {
        let mut p=Probe::new();p.next=u32::MAX-1;p.stage(info,||sample(T));assert_eq!(p.pending().unwrap().sequence,u32::MAX-1);p.accepted();
        p.stage(forbidden_info,forbidden_read);let terminal=p.pending().unwrap();assert_eq!(terminal.sequence,u32::MAX);assert_eq!(terminal.state,State::Exhausted);assert_eq!(terminal.seconds,0);
        assert!(!p.stage(forbidden_info,forbidden_read));assert_eq!(p.pending(),Some(terminal));p.accepted();assert!(!p.stage(forbidden_info,forbidden_read));assert_eq!(p.pending(),None);
    }
}
