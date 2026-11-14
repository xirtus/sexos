// SPDX-License-Identifier: GPL-3.0-only
//! Host tests for the voice: `cargo test -p sex-speech`.
use sex_speech::{speak, stop, Spoken, SAMPLE_RATE};

fn say(text: &str, wpm: u16) -> (Vec<i16>, Spoken) {
    let mut pcm = Vec::new();
    let spoken = speak(text, wpm, &mut |s: &[i16]| pcm.extend_from_slice(s)).expect("voice available");
    assert_eq!(spoken.samples, pcm.len());
    (pcm, spoken)
}

fn rms(pcm: &[i16]) -> f64 {
    (pcm.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / pcm.len().max(1) as f64).sqrt()
}

#[test]
fn ok_button_is_speech_of_plausible_length() {
    let (pcm, spoken) = say("OK button", 175);
    let secs = pcm.len() as f64 / SAMPLE_RATE as f64;
    assert!((0.3..=2.0).contains(&secs), "duration {secs} s");
    assert!(!spoken.stopped);
    let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap();
    assert!(peak > 3000, "peak {peak}");
    assert!(rms(&pcm) > 300.0, "rms {}", rms(&pcm));
    // Speech, not a tone: loud 10 ms frames and quiet ones (the gap, the stop in "button").
    let frames: Vec<f64> = pcm.chunks(SAMPLE_RATE as usize / 100).map(rms).collect();
    let loud = frames.iter().filter(|&&r| r > 1000.0).count();
    let quiet = frames.iter().filter(|&&r| r < 100.0).count();
    assert!(loud >= 15 && quiet >= 3, "loud {loud} quiet {quiet} of {}", frames.len());
}

#[test]
fn empty_text_is_silent() {
    for text in ["", "   ", "\n\t", "\0hidden"] {
        let (pcm, spoken) = say(text, 175);
        assert!(pcm.is_empty(), "{text:?} gave {} samples", pcm.len());
        assert_eq!(spoken, Spoken::default());
    }
}

#[test]
fn output_is_deterministic() {
    let (a, _) = say("OK button", 175);
    // Whatever was spoken in between (another text, another rate) does not matter.
    let _ = say("Cancel. Dialog, Save changes?", 260);
    let _ = say("OK button", 400);
    let mut cut = 0;
    let _ = speak("A long sentence that is cut off before it ends, as focus moves.", 175, &mut |s: &[i16]| {
        cut += s.len();
        if cut > 4000 { stop() }
    });
    let (b, _) = say("OK button", 175);
    assert_eq!(a, b);
    let (c, _) = say("OK button", 175);
    assert_eq!(b, c);
}

#[test]
fn faster_rate_is_shorter() {
    let (slow, _) = say("The quick brown fox jumps over the lazy dog", 120);
    let (fast, _) = say("The quick brown fox jumps over the lazy dog", 300);
    assert!(fast.len() * 3 < slow.len() * 2, "slow {} fast {}", slow.len(), fast.len());
}

#[test]
fn stop_ends_the_utterance_early() {
    let text = "This is a long sentence that the screen reader starts to read, and then the focus moves on to something else entirely.";
    let (full, _) = say(text, 175);
    let mut pcm = Vec::new();
    let spoken = speak(text, 175, &mut |s: &[i16]| {
        pcm.extend_from_slice(s);
        if pcm.len() >= SAMPLE_RATE as usize / 2 { stop() }
    }).unwrap();
    assert!(spoken.stopped);
    assert!(pcm.len() < full.len() / 2, "stopped at {} of {}", pcm.len(), full.len());
    // The next utterance is whole again.
    let (again, spoken) = say("OK button", 175);
    assert!(!spoken.stopped && again.len() > SAMPLE_RATE as usize * 3 / 10);
}

#[test]
fn words_numbers_and_unicode_speak() {
    for text in ["Save changes? 3 files, 12.5 MB", "naïve café — ünïcode ✓", "Ctrl+Alt+Delete"] {
        let (pcm, _) = say(text, 200);
        assert!(rms(&pcm) > 300.0, "{text:?}");
    }
}
