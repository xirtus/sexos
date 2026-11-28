// SPDX-License-Identifier: GPL-3.0-only
// The live recorder (MediaRecorder's encoder): what it writes is decoded back with
// sex-media's own Decoder. Needs the encoders: run with SEXOS_MEDIA_HOST_FFMPEG=1 and
// the host's FFmpeg development files (libvpx, Opus), as CI does
// (the Host regressions job). Without them only the pacing and the refusals
// are checked.
use sex_media::live::{self, Config, Live, Pacer, Video};
use sex_media::{Decoder, Next};

fn encoders() -> bool {
    let want = std::env::var("SEXOS_MEDIA_HOST_FFMPEG").is_ok_and(|v| v == "1");
    let have = live::supported(Some(Video::Vp8), true);
    assert!(!want || have, "SEXOS_MEDIA_HOST_FFMPEG=1 but VP8 + Opus recording is unavailable");
    have
}

fn config(video: Option<Video>, w: u32, h: u32, audio: bool) -> Config {
    Config { video, width: w, height: h, fps: 30, video_bps: 1_000_000, audio, audio_bps: 96_000 }
}

/// A picture that moves: a red field with a white box at a place that depends on `n`.
fn picture(w: usize, h: usize, n: usize) -> Vec<u32> {
    let mut px = vec![0x00C0_2020u32; w * h];
    let x0 = (n * 4) % (w - 16);
    for y in 4..20 { for x in x0..x0 + 16 { px[y * w + x] = 0x00FF_FFFF; } }
    px
}

fn tone(from: u64, n: usize) -> Vec<[i16; 2]> {
    (0..n as u64).map(|i| { let t = from + i; let v = if (t / 54) % 2 == 0 { 6000 } else { -6000 }; [v, v] }).collect()
}

struct Played { pictures: usize, sound: usize, size: (u32, u32), video: String, audio: String, format: String, last_ms: i64, loud: bool }

fn play(bytes: Vec<u8>) -> Played {
    let mut d = Decoder::open(Box::new(bytes)).expect("the recording opens");
    let info = d.info.clone();
    let (w, h) = (info.width.max(1) as usize, info.height.max(1) as usize);
    let mut out = vec![0u32; w * h];
    let mut pcm = vec![[0i16; 2]; 8192];
    let (mut pictures, mut sound, mut last_ms, mut loud) = (0, 0, -1, false);
    loop {
        match d.next(&mut out, w, h, &mut pcm).expect("decodes") {
            Next::Picture { at_ms } => { pictures += 1; last_ms = last_ms.max(at_ms); }
            Next::Sound { frames, .. } => { sound += frames; loud |= pcm[..frames].iter().any(|s| s[0].unsigned_abs() > 2000); }
            Next::Later | Next::End => break,
        }
    }
    Played { pictures, sound, size: (info.width, info.height), video: info.video_codec.to_string(), audio: info.audio_codec.to_string(),
             format: info.format.to_string(), last_ms, loud }
}

/// Three seconds at 30 pictures a second with sound, taken in pieces every half second
/// (MediaRecorder's timeslice): the pieces joined play as one file.
fn record(video: Video, name: &str) {
    if !encoders() { return; }
    let (w, h) = (160usize, 120usize);
    let mut rec = Live::new(config(Some(video), w as u32, h as u32, true)).expect("opens");
    let mut file = Vec::new();
    let mut pieces = 0;
    let first = rec.take();
    assert!(first.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]), "{name}: starts with the EBML header");
    file.extend_from_slice(&first);
    for n in 0..90usize {
        let at = n as u64 * 1000 / 30;
        rec.picture(&picture(w, h, n), w, h, at.max(1)).expect("picture");
        let due = (at + 33) * 48;
        let have = rec.samples;
        if due > have { rec.sound(&tone(have, (due - have) as usize)).expect("sound"); }
        if n % 15 == 14 {
            rec.flush().expect("flush");
            let piece = rec.take();
            assert!(!piece.is_empty(), "{name}: piece {pieces} has bytes");
            file.extend_from_slice(&piece);
            pieces += 1;
        }
    }
    rec.finish().expect("finish");
    file.extend_from_slice(&rec.take());
    assert_eq!(pieces, 6);
    assert_eq!(rec.pictures, 90);
    let p = play(file);
    assert_eq!(p.format, "matroska,webm", "{name}");
    assert_eq!(p.video, name);
    assert_eq!(p.audio, "opus", "{name}");
    assert_eq!(p.size, (w as u32, h as u32), "{name}");
    assert_eq!(p.pictures, 90, "{name}: every picture comes back");
    assert!((2900..=3100).contains(&p.last_ms), "{name}: last picture at {} ms", p.last_ms);
    // Opus keeps 312 samples of encoder delay; about three seconds of sound.
    assert!((140_000..=150_000).contains(&p.sound), "{name}: {} sound frames", p.sound);
    assert!(p.loud, "{name}: the tone is heard");
}

#[test]
fn vp8_and_opus_record_three_seconds_in_pieces() { record(Video::Vp8, "vp8"); }

#[test]
fn vp9_and_opus_record_three_seconds_in_pieces() { record(Video::Vp9, "vp9"); }

#[test]
fn pictures_of_another_size_are_scaled_and_late_times_refused() {
    if !encoders() { return; }
    let mut rec = Live::new(config(Some(Video::Vp8), 320, 240, false)).expect("opens");
    // A canvas that changed size, and a camera's 640x480: both fill the recording.
    rec.picture(&picture(200, 100, 0), 200, 100, 1).unwrap();
    rec.picture(&picture(640, 480, 1), 640, 480, 40).unwrap();
    assert_eq!(rec.picture(&picture(640, 480, 2), 640, 480, 40), Err(live::Error::Invalid), "times must increase");
    rec.picture(&picture(640, 480, 3), 640, 480, 80).unwrap();
    // Sound to a recording without sound is refused.
    assert_eq!(rec.sound(&tone(0, 10)), Err(live::Error::Invalid));
    rec.finish().unwrap();
    let p = play(rec.take());
    assert_eq!((p.pictures, p.size, p.sound), (3, (320, 240), 0));
}

#[test]
fn sound_only_and_silence_fill() {
    if !encoders() { return; }
    let mut rec = Live::new(config(None, 0, 0, true)).expect("opens");
    // A muted microphone: silence as long as the recording.
    rec.sound_until(1000).unwrap();
    assert_eq!(rec.samples, 48_000);
    rec.finish().unwrap();
    let p = play(rec.take());
    assert_eq!((p.pictures, p.audio.as_str(), p.loud), (0, "opus", false));
    assert!((47_000..=49_000).contains(&p.sound), "{}", p.sound);
}

#[test]
fn bad_configs_are_refused() {
    assert!(matches!(Live::new(config(None, 0, 0, false)), Err(live::Error::Invalid)));
    assert!(matches!(Live::new(config(Some(Video::Vp8), 15, 16, true)), Err(live::Error::Invalid)));
    assert!(matches!(Live::new(config(Some(Video::Vp8), 17, 16, true)), Err(live::Error::Invalid)));
    if !live::supported(Some(Video::Vp8), true) {
        assert!(matches!(Live::new(config(Some(Video::Vp8), 64, 64, true)), Err(live::Error::Unavailable)));
    }
    assert_eq!(live::size_for(1920, 1080, 1280, 720), (1280, 720));
    assert_eq!(live::size_for(301, 151, 1280, 720), (300, 150));
    assert_eq!(live::size_for(0, 0, 1280, 720), (640, 480));
}

#[test]
fn pacer_spaces_pictures_out_when_encoding_falls_behind() {
    let mut p = Pacer::new(30);
    assert_eq!(p.interval_ms(), 33);
    assert!(p.due(0));
    p.spent(0, 5);
    assert!(!p.due(10));
    assert!(p.due(33));
    // Encoding now takes 60 ms a picture: pictures come at most every 120 ms or so.
    let mut at = 33;
    for _ in 0..20 { p.spent(at, 60); at += p.interval_ms() as u64; }
    assert!((110..=125).contains(&p.interval_ms()), "{}", p.interval_ms());
    assert!(!p.due(at - p.interval_ms() as u64 + 40));
    assert!(p.skipped > 0);
    // Never slower than two a second.
    for _ in 0..20 { p.spent(at, 2000); }
    assert_eq!(p.interval_ms(), Pacer::SLOWEST_MS);
    // It catches up again once encoding is quick.
    for _ in 0..40 { p.spent(at, 1); }
    assert_eq!(p.interval_ms(), 33);
}
