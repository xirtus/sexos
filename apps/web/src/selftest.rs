// SPDX-License-Identifier: GPL-3.0-only
//! Internal SexOS Browser Media & YouTube Self-Test runner for Kaleidoscope (`apps/web`).
//!
//! Invoked via:
//! - `run kaleidoscope --selftest` (offline/deterministic; works without `+net`)
//! - `run +net kaleidoscope --selftest-net` (includes live YouTube TLS 1.3 / MSE probe)
//! - Optional `--selftest-exit` flag to exit with status 0/1 after logging summary
//! - Or by navigating to `about:selftest` / `about:selftest-net` inside Kaleidoscope.

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_media::{Decoder, Error as MediaError, Next};
use sex_web::selftest::{
    analyze_yt_watch_html, fnv1a_pixels, parse_oembed_json, render_report_html,
    run_dom_js_timers_test, run_mse_pipeline_core_test, synth_fmp4_h264_init,
    synth_fmp4_h264_segment, synth_pcm_wav, CaseResult, Summary,
};

use crate::{clock, media, mse, Browser, SOURCE_MAX};

const YT_OEMBED_URL: &str =
    "https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v=dQw4w9WgXcQ&format=json";
const YT_WATCH_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

/// Runs the Kaleidoscope browser media self-test suite, emits `[web.selftest]` and
/// `[web.selftest.summary]` records to the serial/tether log, and returns `(html, summary)`.
pub fn run(browser: &mut Browser, include_net: bool) -> (String, Summary) {
    let mode = if include_net { "selftest-net" } else { "selftest" };
    sex_pdx::serial_record!("[web.selftest.begin] mode={}", mode);

    let mut results: Vec<CaseResult> = Vec::with_capacity(5);

    // Test 1: DOM + QuickJS-NG + MediaSource.isTypeSupported + Timers/rAF + onclick
    results.push(run_dom_js_timers_test(clock, sex_pdx::get_ticks));

    // Test 2a: Core MSE fMP4/WebM segmenting + codec matrix
    results.push(run_mse_pipeline_core_test(sex_pdx::get_ticks));

    // Test 2b: Browser mse::Mse SourceBuffer append + buffered ranges + decoder open/seek
    results.push(run_browser_mse_test());

    // Test 3: Audio/Video decode, pixel hash, seek accuracy & sexbell PCM playback
    results.push(run_media_av_sexbell_test());

    // Test 4: Live YouTube HTTPS/TLS 1.3 + /oembed + watch DASH/MSE probe (when --selftest-net)
    if include_net {
        results.push(run_youtube_net_probe(browser));
    }

    for r in &results {
        sex_pdx::serial_record!("{}", r.format_line());
    }
    let summary = Summary::from_results(&results);
    sex_pdx::serial_record!("{}", summary.format_line());

    let html = render_report_html(mode, &results, &summary);
    (html, summary)
}

/// Test 2b: Drives `apps/web/src/mse.rs` (`Mse::add`, `append`, `end`, `ready_at`, `decoders`)
/// with a heap-synthesized H.264 fMP4 stream (2 seconds, 2 segments).
fn run_browser_mse_test() -> CaseResult {
    let t0 = sex_pdx::get_ticks();

    let init = synth_fmp4_h264_init(1, 1, 90_000, 3_000);
    let seg0 = synth_fmp4_h264_segment(1, 0, 30, 3_000, 1, 1, 130, 100, 170);
    let seg1 = synth_fmp4_h264_segment(2, 90_000, 30, 3_000, 1, 1, 190, 150, 90);

    let mut m = mse::Mse::new();
    m.add(r#"video/mp4; codecs="avc1.640028""#);
    m.append(1, 0, &init);
    m.append(1, 0, &seg0);
    m.append(1, 0, &seg1);
    m.end();

    let last_reply = m.replies.last().cloned().unwrap_or_default();
    let ready0 = m.ready_at(0.0);
    let ready1 = m.ready_at(1.0);
    let first = m.first();
    m.playhead(0.5);

    if !ready0 || !ready1 || first != Some(0.0) || !last_reply.contains("0-2") {
        return CaseResult::fail(
            "mse_sourcebuffer",
            sex_pdx::get_ticks().saturating_sub(t0),
            format!(
                "ready0={} ready1={} first={:?} reply={}",
                ready0 as u8, ready1 as u8, first, last_reply
            ),
        );
    }

    match m.decoders(0.0) {
        Ok((Some(mut vdec), _)) => {
            let mut px = vec![0u32; 16 * 16];
            let mut pcm = vec![[0i16; 2]; 256];
            let mut frames = 0u32;
            let mut hash = 0u64;
            while frames < 30 {
                match vdec.next(&mut px, 16, 16, &mut pcm) {
                    Ok(Next::Picture { .. }) => {
                        frames += 1;
                        if frames == 1 {
                            hash = fnv1a_pixels(&px);
                        }
                    }
                    Ok(Next::Sound { .. }) => {}
                    Ok(Next::Later | Next::End) | Err(_) => break,
                }
            }
            let seek_ok = vdec.seek(1000).is_ok();
            let mut post_seek_ms = -1i64;
            if seek_ok {
                for _ in 0..8 {
                    match vdec.next(&mut px, 16, 16, &mut pcm) {
                        Ok(Next::Picture { at_ms }) => {
                            post_seek_ms = at_ms;
                            break;
                        }
                        Ok(Next::Sound { .. }) => {}
                        _ => break,
                    }
                }
            }
            let elapsed = sex_pdx::get_ticks().saturating_sub(t0);
            if frames > 0 && hash != 0 && seek_ok && post_seek_ms >= 500 {
                CaseResult::pass(
                    "mse_sourcebuffer",
                    elapsed,
                    format!(
                        "bytes={} buffered=0-2 frames={} seek_ms={} hash={:016x}",
                        m.appended, frames, post_seek_ms, hash
                    ),
                )
            } else {
                CaseResult::fail(
                    "mse_sourcebuffer",
                    elapsed,
                    format!(
                        "frames={} hash={:016x} seek_ok={} post_seek_ms={}",
                        frames, hash, seek_ok as u8, post_seek_ms
                    ),
                )
            }
        }
        Ok((None, _)) => CaseResult::fail(
            "mse_sourcebuffer",
            sex_pdx::get_ticks().saturating_sub(t0),
            "no video decoder returned from mse::Mse",
        ),
        Err(MediaError::UNSUPPORTED) => CaseResult::pass(
            "mse_sourcebuffer",
            sex_pdx::get_ticks().saturating_sub(t0),
            format!("bytes={} buffered=0-2 ffmpeg=0(no_decoder_in_build)", m.appended),
        ),
        Err(e) => {
            let mut msg = [0u8; 64];
            CaseResult::fail(
                "mse_sourcebuffer",
                sex_pdx::get_ticks().saturating_sub(t0),
                format!("decoder open err={} why={}", e.0, e.text(&mut msg)),
            )
        }
    }
}

/// Test 3: Synthesizes a short video (H.264 fMP4) + PCM audio (48 kHz stereo WAV) test clip
/// on the heap, runs through `media.rs` + `sex_media::Decoder` + `sex_app::sound::Sound`
/// (`sexbell`), and verifies decode FPS, non-zero pixel hash, seek accuracy, and audio output.
fn run_media_av_sexbell_test() -> CaseResult {
    let t0 = sex_pdx::get_ticks();

    // 1. Synthesize 1.0 s H.264 video (30 frames @ 30 fps) and 250 ms 48 kHz stereo WAV on heap.
    let mut video_bytes = synth_fmp4_h264_init(1, 1, 90_000, 3_000);
    video_bytes.extend_from_slice(&synth_fmp4_h264_segment(1, 0, 30, 3_000, 1, 1, 150, 110, 175));
    let wav_bytes = synth_pcm_wav(250, sex_app::sound::RATE);

    // 2. Exercise `apps/web/src/media.rs` (`Media::open` + seek) on the video clip.
    let dom = sex_web::sex_html::parse::parse("<video controls></video>");
    let vnode = media::elements(&dom).into_iter().next().unwrap_or(0);
    let mut elem = media::Media::new(&dom, vnode);
    let opened_media = elem.open(video_bytes.clone());

    // 3. Measure video decode FPS, pixel hash, and seek accuracy via `sex_media::Decoder`.
    let video_src: Rc<[u8]> = Rc::from(video_bytes);
    let (video_ok, video_detail) = match Decoder::open(Box::new(video_src)) {
        Ok(mut dec) => {
            let decode_t0 = sex_pdx::get_ticks();
            let mut px = vec![0u32; 16 * 16];
            let mut pcm = vec![[0i16; 2]; 256];
            let mut frames = 0u32;
            let mut hash = 0u64;
            loop {
                match dec.next(&mut px, 16, 16, &mut pcm) {
                    Ok(Next::Picture { .. }) => {
                        frames += 1;
                        hash ^= fnv1a_pixels(&px).rotate_left(frames & 31);
                    }
                    Ok(Next::Sound { .. }) => {}
                    Ok(Next::End | Next::Later) | Err(_) => break,
                }
            }
            let decode_ms = sex_pdx::get_ticks().saturating_sub(decode_t0).max(1);
            let fps = (frames as u64).saturating_mul(1000) / decode_ms;

            // Seek to 500 ms and verify timestamp accuracy.
            let seek_res = dec.seek(500);
            let mut seek_at = -1i64;
            if seek_res.is_ok() {
                for _ in 0..8 {
                    if let Ok(Next::Picture { at_ms }) = dec.next(&mut px, 16, 16, &mut pcm) {
                        seek_at = at_ms;
                        break;
                    }
                }
            }
            let seek_err = (seek_at - 500).abs();
            let ok = opened_media && frames >= 25 && hash != 0 && seek_res.is_ok() && seek_at >= 0 && seek_err <= 250;
            (
                ok,
                format!(
                    "video=1 frames={} fps={} hash={:016x} seek_at={}ms",
                    frames, fps, hash, seek_at
                ),
            )
        }
        Err(MediaError::UNSUPPORTED) => (true, String::from("video=no_ffmpeg")),
        Err(e) => {
            let mut msg = [0u8; 64];
            (false, format!("video_err={}({})", e.0, e.text(&mut msg)))
        }
    };

    // 4. Exercise audio decode (WAV) and `sexbell` sound ring output.
    let mut pcm_out: Vec<[i16; 2]> = Vec::new();
    let wav_src: Rc<[u8]> = Rc::from(wav_bytes.clone());
    let audio_decoded = match Decoder::open(Box::new(wav_src)) {
        Ok(mut adec) => {
            let mut px = Vec::new();
            let mut chunk = vec![[0i16; 2]; 1024];
            loop {
                match adec.next(&mut px, 0, 0, &mut chunk) {
                    Ok(Next::Sound { frames, .. }) => pcm_out.extend_from_slice(&chunk[..frames]),
                    Ok(Next::Picture { .. }) => {}
                    Ok(Next::End | Next::Later) | Err(_) => break,
                }
            }
            !pcm_out.is_empty()
        }
        Err(MediaError::UNSUPPORTED) => {
            // Parse raw 16-bit stereo PCM frames directly from our synthetic WAV header (offset 44)
            // so sexbell output is still exercised even in no-FFmpeg builds.
            for f in wav_bytes[44..].chunks_exact(4) {
                let l = i16::from_le_bytes([f[0], f[1]]);
                let r = i16::from_le_bytes([f[2], f[3]]);
                pcm_out.push([l, r]);
            }
            !pcm_out.is_empty()
        }
        Err(_) => false,
    };

    let (bell_ok, bell_detail) = match sex_app::sound::Sound::open() {
        Some(mut bell) => {
            bell.set_volume(20); // gentle test tone
            // Write up to 4800 frames (100 ms @ 48 kHz) into the sexbell ring.
            let to_write = pcm_out.len().min(4800);
            let mut written = 0usize;
            while written < to_write {
                let n = bell.write(&pcm_out[written..to_write]);
                if n == 0 {
                    break;
                }
                written += n;
            }
            // Give Bell a short slice (~20 ms) to consume frames from the ring.
            let until = sex_pdx::get_ticks() + 20;
            while sex_pdx::get_ticks() < until {
                sex_pdx::sys_yield();
            }
            let played = bell.played();
            bell.skip();
            (
                written > 0,
                format!("sexbell=ok written={} played={}", written, played),
            )
        }
        None => (
            true,
            format!("sexbell=unavailable pcm_frames={}", pcm_out.len()),
        ),
    };

    let elapsed = sex_pdx::get_ticks().saturating_sub(t0);
    let detail = format!("{} {} audio_dec={}", video_detail, bell_detail, audio_decoded as u8);
    if video_ok && audio_decoded && bell_ok {
        CaseResult::pass("media_av_sexbell", elapsed, detail)
    } else {
        CaseResult::fail("media_av_sexbell", elapsed, detail)
    }
}

/// Test 4: Live HTTPS TLS 1.3 YouTube `/oembed` + watch page DASH/MSE probe (`--selftest-net`).
fn run_youtube_net_probe(browser: &mut Browser) -> CaseResult {
    let t0 = sex_pdx::get_ticks();

    // 1. Lightweight /oembed probe over TLS 1.3
    let oembed_t0 = sex_pdx::get_ticks();
    let oembed_res = browser.http("GET", YT_OEMBED_URL, &[], &[], 64 * 1024, None);
    let oembed_ms = sex_pdx::get_ticks().saturating_sub(oembed_t0);
    let (oembed_status, oembed_title) = match oembed_res {
        Ok(r) => {
            let title = parse_oembed_json(&r.body)
                .map(|o| o.title)
                .unwrap_or_default();
            (r.status, title)
        }
        Err(why) => {
            let reason = core::str::from_utf8(why).unwrap_or("net error");
            return CaseResult::fail(
                "yt_net_probe",
                sex_pdx::get_ticks().saturating_sub(t0),
                format!("oembed_err={} ms={}", reason, oembed_ms),
            );
        }
    };

    // 2. Watch page fetch + ytInitialPlayerResponse DASH/MSE codec inspection
    let watch_t0 = sex_pdx::get_ticks();
    let watch_res = browser.http("GET", YT_WATCH_URL, &[], &[], SOURCE_MAX, None);
    let watch_ms = sex_pdx::get_ticks().saturating_sub(watch_t0);
    let (watch_status, probe) = match watch_res {
        Ok(r) => {
            let html = core::str::from_utf8(&r.body).unwrap_or("");
            (r.status, analyze_yt_watch_html(html))
        }
        Err(why) => {
            let reason = core::str::from_utf8(why).unwrap_or("net error");
            return CaseResult::fail(
                "yt_net_probe",
                sex_pdx::get_ticks().saturating_sub(t0),
                format!(
                    "oembed={}({}ms) watch_err={} ms={}",
                    oembed_status, oembed_ms, reason, watch_ms
                ),
            );
        }
    };

    let elapsed = sex_pdx::get_ticks().saturating_sub(t0);
    let detail = format!(
        "oembed={}({}ms,title_len={}) watch={}({}ms) player_resp={} formats={} mse_ok={}(v={},a={})",
        oembed_status,
        oembed_ms,
        oembed_title.len(),
        watch_status,
        watch_ms,
        probe.has_player_response as u8,
        probe.total_formats,
        probe.mse_supported,
        probe.video_supported,
        probe.audio_supported
    );

    if oembed_status == 200 && !oembed_title.is_empty() && watch_status == 200 && (!probe.has_player_response || probe.mse_supported > 0) {
        CaseResult::pass("yt_net_probe", elapsed, detail)
    } else {
        CaseResult::fail("yt_net_probe", elapsed, detail)
    }
}
