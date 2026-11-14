// SPDX-License-Identifier: GPL-3.0-only
//! Guest engine proof, not Moan's UI. Two edited tracks mixed into Bell's ring.
#![no_std]
#![no_main]
extern crate alloc;
use moan_core::{Clip, Track, RATE};
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[moan.probe.fail] {}", info);
    sex_pdx::task_finish(1)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut sound = sex_app::sound::Sound::open().expect("sound ring");
    sound.set_rate(RATE);
    // Integer square waves with exact periods, two seconds of source.
    let left: alloc::vec::Vec<_> = (0..RATE * 2)
        .map(|i| [if i % 120 < 60 { 6000 } else { -6000 }; 2])
        .collect();
    let right: alloc::vec::Vec<_> = (0..RATE * 2)
        .map(|i| [if i % 80 < 40 { 6000 } else { -6000 }; 2])
        .collect();
    let mut a = Track::new();
    a.set_pan(-100).unwrap();
    let mut clip = Clip::new(&left, 0).unwrap();
    let rest = clip.split(RATE as usize).unwrap();
    a.clips.extend([clip, rest]);
    let mut b = Track::new();
    b.set_pan(100).unwrap();
    b.set_volume(50).unwrap();
    let mut clip = Clip::new(&right, 0).unwrap();
    clip.set_gain(200).unwrap();
    clip.set_fades(480, 480).unwrap();
    b.clips.push(clip);
    let tracks = [a, b];
    let mut at = 0;
    let mut block = [[0i16; 2]; 512];
    let deadline = sex_pdx::get_ticks() + 30_000;
    while at < RATE as usize * 2 {
        assert!(sex_pdx::get_ticks() < deadline, "sound write timeout");
        let n = block.len().min(RATE as usize * 2 - at).min(sound.room());
        moan_core::render(&tracks, at, &mut block[..n]);
        at += sound.write(&block[..n]);
        if n == 0 {
            let _ = sex_pdx::sys_wait_self_ticks(4);
        }
    }
    sound.close();
    while sound.playing() {
        assert!(sex_pdx::get_ticks() < deadline, "sound drain timeout");
        let _ = sex_pdx::sys_wait_self_ticks(4);
    }
    sex_pdx::serial_record!("[moan.probe.ok] frames={} played={}", at, sound.played());
    sex_pdx::task_finish(0)
}
