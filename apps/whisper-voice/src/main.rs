// SPDX-License-Identifier: GPL-3.0-only
//! The voice of Whisper, the screen reader: a confined program that turns text into
//! speech (eSpeak NG, through crates/sex-speech) and plays it through its own sound
//! ring. It holds no files, no network and no other program's anything: Shell, which
//! runs the screen reader, sends it text (`sex_pdx::speech_voice`, opcode 0x2DE) and
//! it speaks. A new utterance cuts off the one before, as a screen reader should.
//!
//! `run whisper-voice OK button` speaks its arguments once at start, for trying the
//! voice and for the gate. It then waits for Shell until asked to stop.
//!
//! Serial record: `[whisper.voice.ready] rate=22050`, then for each utterance
//! `[whisper.voice.text] TEXT` and `[whisper.voice.spoke] samples=N ms=N peak=N
//! stopped=0|1 hash=H` (hash: FNV-1a over the mono samples).
use sex_std as _;
use sex_libc as _;

#[cfg(target_os = "hermit")]
mod voice {
    use sex_speech::{speak, stop, DEFAULT_WPM, SAMPLE_RATE};
    use sex_std::sex_app::{mailbox, sound::Sound};
    use sex_std::sex_pdx::{self as pdx, app_lifecycle as lifecycle, program_store as store, speech_voice as wire, PdxMessage};
    use std::collections::VecDeque;

    /// Shell's PD, the only sender the voice listens to.
    const SHELL: u32 = 3;

    pub struct Voice {
        sound: Option<Sound>,
        gather: wire::Gather,
        /// Shell's pieces that arrived while an utterance was being spoken.
        waiting: VecDeque<[u64; 3]>,
        me: (u32, u32),
        quit: bool,
    }

    impl Voice {
        pub fn open() -> Voice {
            let mut sound = Sound::open();
            if let Some(s) = sound.as_mut() { s.set_rate(SAMPLE_RATE); }
            if sound.is_none() { pdx::serial_record!("[whisper.voice.sound.none]"); }
            Voice { sound, gather: wire::Gather::new(), waiting: VecDeque::new(), me: sex_std::identity(), quit: false }
        }

        /// Takes what arrived for the voice without waiting. Messages for other tasks
        /// (file answers) go to the mailbox for their owner.
        fn take_messages(&mut self) {
            let ours = |m: &PdxMessage| m.caller_pd == SHELL;
            let handle = |v: &mut Voice, m: PdxMessage| {
                if m.type_id == wire::OP_SPEAK {
                    // A new utterance stops the current one now, not when it is reached.
                    if wire::Gather::starts(m.arg0) { stop(); }
                    v.waiting.push_back([m.arg0, m.arg1, m.arg2]);
                } else if lifecycle::close_requested(m.type_id, m.caller_pd, [m.arg0, m.arg1, m.arg2], v.me.0, v.me.1) {
                    v.quit = true;
                    stop();
                }
            };
            while let Some(m) = mailbox::take(ours) { handle(self, m); }
            while let Some(m) = pdx::pdx_try_listen_raw(0) {
                if ours(&m) { handle(self, m) } else { mailbox::put(m) }
            }
            if store::record_call(store::RECORD_STOP_REQUESTED, 0, 0).1 != 0 { self.quit = true; stop(); }
        }

        /// Speaks `text`, playing it as it is made. Returns early when Shell sends
        /// a new utterance or the program is asked to end.
        pub fn say(&mut self, text: &str, wpm: u16) {
            pdx::serial_record!("[whisper.voice.text] {}", text);
            let (mut peak, mut hash) = (0u16, 0xcbf2_9ce4_8422_2325u64);
            let mut frames: Vec<[i16; 2]> = Vec::new();
            let mut sound = self.sound.take();
            let result = speak(text, wpm, &mut |pcm: &[i16]| {
                for &s in pcm {
                    peak = peak.max(s.unsigned_abs());
                    hash = (hash ^ s as u16 as u64).wrapping_mul(0x100_0000_01b3);
                }
                self.take_messages();
                let Some(sound) = sound.as_mut() else { return };
                frames.clear();
                frames.extend(pcm.iter().map(|&s| [s, s]));
                let mut at = 0;
                while at < frames.len() {
                    at += sound.write(&frames[at..]);
                    if at < frames.len() {
                        let _ = pdx::sys_wait_self_ticks(5);
                        self.take_messages();
                        if self.quit || self.waiting.front().is_some_and(|p| wire::Gather::starts(p[0])) { return; }
                    }
                }
            });
            match result {
                Ok(spoken) => {
                    if spoken.stopped {
                        // Drop what is queued but not yet heard.
                        if let Some(s) = sound.as_mut() { s.skip(); }
                    }
                    pdx::serial_record!("[whisper.voice.spoke] samples={} ms={} peak={} stopped={} hash={:016x}",
                        spoken.samples, spoken.ms(), peak, spoken.stopped as u8, hash);
                }
                Err(e) => pdx::serial_record!("[whisper.voice.failed] {:?}", e),
            }
            self.sound = sound;
        }

        /// Serves Shell until asked to end.
        pub fn serve(&mut self) -> ! {
            loop {
                self.take_messages();
                if self.quit { break; }
                let Some([a, b, c]) = self.waiting.pop_front() else {
                    let _ = pdx::sys_wait_self_ticks(10);
                    continue;
                };
                let request = match self.gather.push(a, b, c) {
                    Some(wire::Request::Speak { text, wpm }) => Some((String::from_utf8_lossy(text).into_owned(), wpm)),
                    Some(wire::Request::Stop) => {
                        if let Some(s) = self.sound.as_mut() { s.skip(); }
                        pdx::serial_record!("[whisper.voice.hushed]");
                        None
                    }
                    None => None,
                };
                if let Some((text, wpm)) = request {
                    self.say(&text, if wpm == 0 { DEFAULT_WPM } else { wpm });
                }
            }
            if let Some(s) = self.sound.as_mut() {
                s.close();
                // Let the last words finish, but not forever.
                let end = pdx::get_ticks() + 3000;
                while s.pending() > 0 && pdx::get_ticks() < end { let _ = pdx::sys_wait_self_ticks(10); }
            }
            pdx::serial_record!("[whisper.voice.done]");
            pdx::task_finish(0)
        }
    }
}

fn main() {
    #[cfg(target_os = "hermit")]
    {
        let mut v = voice::Voice::open();
        sex_std::sex_pdx::serial_record!("[whisper.voice.ready] rate={} data_bytes={}", sex_speech::SAMPLE_RATE, sex_speech::data_bytes());
        let words: Vec<String> = std::env::args().skip(1).collect();
        if !words.is_empty() { v.say(&words.join(" "), sex_speech::DEFAULT_WPM); }
        v.serve();
    }
    #[cfg(not(target_os = "hermit"))]
    {
        // On the host: speak the arguments into a WAV file, for listening to the voice.
        let words: Vec<String> = std::env::args().skip(1).collect();
        let text = if words.is_empty() { "OK button".to_string() } else { words.join(" ") };
        let mut pcm: Vec<i16> = Vec::new();
        match sex_speech::speak(&text, sex_speech::DEFAULT_WPM, &mut |s: &[i16]| pcm.extend_from_slice(s)) {
            Ok(spoken) => {
                let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
                let data = (pcm.len() * 2) as u32;
                wav.extend_from_slice(b"RIFF"); wav.extend_from_slice(&(36 + data).to_le_bytes());
                wav.extend_from_slice(b"WAVEfmt "); wav.extend_from_slice(&16u32.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes()); wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&sex_speech::SAMPLE_RATE.to_le_bytes()); wav.extend_from_slice(&(sex_speech::SAMPLE_RATE * 2).to_le_bytes());
                wav.extend_from_slice(&2u16.to_le_bytes()); wav.extend_from_slice(&16u16.to_le_bytes());
                wav.extend_from_slice(b"data"); wav.extend_from_slice(&data.to_le_bytes());
                for s in &pcm { wav.extend_from_slice(&s.to_le_bytes()); }
                let _ = std::fs::write("whisper-voice.wav", &wav);
                println!("{} ms of speech in whisper-voice.wav", spoken.ms());
            }
            Err(e) => println!("no voice: {e:?}"),
        }
    }
}
