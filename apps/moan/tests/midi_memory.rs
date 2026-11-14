use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
struct Count;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FAIL_ABOVE: AtomicUsize = AtomicUsize::new(usize::MAX);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        if l.size() >= FAIL_ABOVE.load(Ordering::Relaxed) {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        if n >= FAIL_ABOVE.load(Ordering::Relaxed) {
            return core::ptr::null_mut();
        }
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static ALLOC: Count = Count;
#[test]
fn twelve_undo_entries_store_notes_not_long_pcm_buffers() {
    let mut s = moan::Session::default();
    let mut pattern = moan_core::midi::Pattern::preset(moan_core::midi::Instrument::SilkTriangle);
    pattern.frames = moan_core::RATE * 16;
    s.add_midi(pattern).unwrap();
    s.mark_saved();
    let fingerprint = s.midi_clip().unwrap().midi_pattern().unwrap().fingerprint();
    let cache = s.midi_clip().unwrap().source_frames().as_ptr();
    FAIL_ABOVE.store(1024 * 1024, Ordering::Relaxed);
    let failed = s.edit_midi(|p| p.notes[0].key = 70);
    FAIL_ABOVE.store(usize::MAX, Ordering::Relaxed);
    assert!(failed.is_err());
    assert!(!s.is_dirty());
    assert_eq!(
        s.midi_clip().unwrap().midi_pattern().unwrap().fingerprint(),
        fingerprint
    );
    assert_eq!(s.midi_clip().unwrap().source_frames().as_ptr(), cache);
    let before = LIVE.load(Ordering::Relaxed);
    for n in 0..12 {
        s.edit_midi(|p| p.notes[0].key = 60 + n).unwrap();
    }
    let growth = LIVE.load(Ordering::Relaxed) - before;
    assert!(
        growth < 128 * 1024,
        "MIDI undo retained PCM: live growth {growth}"
    );
    assert!(s.undo());
    assert!(s.redo());
    let mut block = [[0; 2]; 256];
    let mut meter = serenade_core::LevelMeter::new(2, 48000, 800, 18.0);
    let mut history = moan::meter_history::History::new(4096);
    let mut consumed_meters =
        core::array::from_fn(|_| serenade_core::LevelMeter::new(2, 48000, 800, 18.0));
    let allocations = ALLOCS.load(Ordering::Relaxed);
    moan_core::render_processed_observed(&mut s.tracks, 0, &mut block, |_, f| {
        meter.process_frame(&[f[0] as f32 / 32768.0, f[1] as f32 / 32768.0])
    });
    let mut offsets = [0usize; moan::MAX_TRACKS];
    moan_core::render_processed_observed(&mut s.tracks, 0, &mut block, |track, frame| {
        history.write(track, offsets[track], frame);
        offsets[track] += 1;
    });
    history.advance(128, block.len(), &mut consumed_meters);
    assert_eq!(
        ALLOCS.load(Ordering::Relaxed),
        allocations,
        "live MIDI render/meter path allocated"
    );
}
