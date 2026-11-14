use moan::meter_history::{fill_budget, History};
use serenade_core::LevelMeter;
fn meters() -> [LevelMeter; moan::MAX_TRACKS] {
    core::array::from_fn(|_| LevelMeter::new(2, 48000, 800, 18.0))
}
#[test]
fn queued_audio_cannot_light_meters_until_consumed() {
    let mut history = History::new(16);
    let mut meters = meters();
    history.reset(100);
    for at in 100..112 {
        history.write(0, at, [16384; 2]);
    }
    assert_eq!(history.advance(100, 112, &mut meters), 0);
    assert_eq!(meters[0].snapshot()[0].peak, 0.0);
    history.advance(104, 112, &mut meters);
    assert_eq!(meters[0].snapshot()[0].peak, 0.5);
    assert_eq!(meters[1].snapshot()[0].peak, 0.0);
}
#[test]
fn seek_wrap_and_overrun_are_bounded_and_reset_old_peaks() {
    let mut history = History::new(8);
    let mut meters = meters();
    history.reset(0);
    history.write(0, 0, [32767; 2]);
    history.advance(1, 1, &mut meters);
    assert!(meters[0].snapshot()[0].peak > 0.99);
    for at in 8..16 {
        history.write(0, at, [8192; 2]);
    }
    assert_eq!(history.advance(16, 16, &mut meters), 7);
    assert_eq!(meters[0].snapshot()[0].held_peak, 0.25);
    // New play clears tracks absent from the next project, including old rings.
    history.reset(500);
    for meter in &mut meters {
        meter.reset();
    }
    history.advance(508, 508, &mut meters);
    assert_eq!(meters[0].snapshot()[0].peak, 0.0);
}
#[test]
fn fill_budget_does_not_chase_a_concurrently_draining_ring() {
    assert_eq!(fill_budget(64512, usize::MAX, 1024), 64512);
    assert_eq!(fill_budget(usize::MAX, usize::MAX, 1024), 65536);
    assert_eq!(fill_budget(64512, 123, 1024), 123);
    assert_eq!(fill_budget(0, 999, 1024), 0);
}
