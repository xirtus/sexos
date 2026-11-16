// SPDX-License-Identifier: GPL-3.0-only
//! Ephemeral one-shot generative worker binary.
//!
//! Executes a single job (CreateSong, AddLayer, ReplaceSection, Stems, etc.)
//! and exits immediately so 100% of generation memory is reclaimed.

#![cfg_attr(all(feature = "sexos", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "sexos", target_os = "none"), no_main)]

extern crate alloc;

#[cfg(not(all(feature = "sexos", target_os = "none")))]
fn main() {
    use moan_gen::{
        ModelPackState, MuseConfig, MuseJobRequest, MuseMode, MuseSupervisor, Nonce, RamTier,
    };

    let config = MuseConfig {
        enabled: true,
        ..Default::default()
    };

    let mut supervisor = MuseSupervisor::new(config, 8192, ModelPackState::FeatherweightOnly);

    let nonce: Nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let mode = MuseMode::CreateSong {
        prompt: alloc::string::String::from("Moan Ephemeral Worker Theme"),
        lyrics: alloc::string::String::from("Synthesizing in pure Rust and exiting to free RAM"),
        vocal_language: alloc::string::String::from("en"),
        bpm: 124,
        key_scale: alloc::string::String::from("A Minor"),
        time_sig: [4, 4],
        duration_frames: 48000 * 2, // 2 seconds
        thinking_cot: false,
        batch_count: 1,
    };

    let req = MuseJobRequest::new(nonce, mode, RamTier::Tier0Featherweight)
        .expect("valid request");

    supervisor.submit_job(req).expect("job completed");
    let result = supervisor.take_result().expect("result available");

    std::println!(
        "[moangen.ok] rendered {} frames, {} stems. Exiting to free 100% RAM.",
        result.frames.len(),
        result.stems.len()
    );
}

#[cfg(all(feature = "sexos", target_os = "none"))]
use sex_app as _;

#[cfg(all(feature = "sexos", target_os = "none"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[moangen.fatal] payload=redacted");
    sex_pdx::task_finish(1)
}

#[cfg(all(feature = "sexos", target_os = "none"))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    use moan_gen::{
        ModelPackState, MuseConfig, MuseJobRequest, MuseMode, MuseSupervisor, Nonce, RamTier,
    };

    sex_pdx::serial_record!("[moangen.start] ephemeral worker running");
    let config = MuseConfig {
        enabled: true,
        ..Default::default()
    };
    let mut supervisor = MuseSupervisor::new(config, 4096, ModelPackState::FeatherweightOnly);

    let nonce: Nonce = [0x50, 0x4c, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let mode = MuseMode::CreateSong {
        prompt: alloc::string::String::from("SexOS Guest Generative Audio"),
        lyrics: alloc::string::String::new(),
        vocal_language: alloc::string::String::from("en"),
        bpm: 120,
        key_scale: alloc::string::String::from("C Major"),
        time_sig: [4, 4],
        duration_frames: 48000 * 2,
        thinking_cot: false,
        batch_count: 1,
    };

    if let Ok(req) = MuseJobRequest::new(nonce, mode, RamTier::Tier0Featherweight) {
        if supervisor.submit_job(req).is_ok() {
            if let Some(res) = supervisor.take_result() {
                sex_pdx::serial_record!(
                    "[moangen.ok] frames={} stems={}",
                    res.frames.len(),
                    res.stems.len()
                );
                sex_pdx::task_finish(0)
            }
        }
    }

    sex_pdx::task_finish(1)
}
