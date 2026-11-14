// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
#[cfg(feature = "sexos")]
mod guest {
    extern crate alloc;
    use alloc::{format, vec, vec::Vec};
    use moan::{Action, Studio};
    use moan_core::{Frame, RATE};
    use sex_app::{files, sound::Sound};
    use weft::{sexos::App, Response, Strings, Theme};
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[moan.fatal] {}", info);
        sex_pdx::task_finish(1)
    }
    fn capture(
        sound: &mut Sound,
        pcm: &mut Vec<Frame>,
        lost: &mut u64,
        block: &mut [Frame],
    ) -> usize {
        let n = sound.hear(block);
        let skipped = sound.lost().saturating_sub(*lost) as usize;
        *lost = sound.lost();
        let gap = skipped.min(moan::MAX_FRAMES.saturating_sub(pcm.len()));
        pcm.resize(pcm.len() + gap, [0; 2]);
        let keep = n.min(moan::MAX_FRAMES.saturating_sub(pcm.len()));
        pcm.extend_from_slice(&block[..keep]);
        n
    }
    /// Drain codec writes in the file owner's main thread. AVIO callbacks never
    /// perform IPC. Ordered header rewrites are as important as appended packets.
    fn export_drain(
        app: &mut App,
        encoder: &mut moan_export::Encoder,
        path: &[u8],
    ) -> Result<bool, &'static str> {
        loop {
            while let Some(event) = app.window.poll() {
                match event {
                    sex_app::Event::Close => {
                        sex_pdx::serial_record!("[moan.export.cancelled] partial_retained=1");
                        sex_pdx::task_finish(0);
                    }
                    sex_app::Event::Resize { width, height } => {
                        app.ui.handle(weft::Event::Resize {
                            width: width as u32,
                            height: height as u32,
                        });
                    }
                    _ => {}
                }
            }
            match encoder
                .output()
                .map_err(|_| "Encoder output failed; partial file retained")?
            {
                moan_export::Drain::NeedFrames => return Ok(false),
                moan_export::Drain::End => return Ok(true),
                moan_export::Drain::Write(write) => {
                    let (token, offset, length) = (write.token, write.offset, write.bytes.len());
                    let wrote = files::write_at(&mut app.window, path, offset, write.bytes)
                        .map_err(|_| "Export write failed; partial file retained")?;
                    if wrote != length {
                        return Err("Short export write; partial file retained");
                    }
                    encoder
                        .acknowledge(token, wrote)
                        .map_err(|_| "Export acknowledgement failed; partial file retained")?;
                }
            }
        }
    }
    fn export(app: &mut App, studio: &mut Studio) -> Result<(), &'static str> {
        let frames = studio
            .session
            .duration()
            .checked_add(RATE as usize * 2)
            .ok_or("Mix too long")?;
        if studio.session.tracks.is_empty() {
            return Err("Add audio first");
        }
        let format = moan_export::Format::try_from(studio.export_format)
            .map_err(|_| "Choose WAV, FLAC or MP3")?;
        // Verify a real codec is available before creating any destination.
        let mut encoder = moan_export::Encoder::new(format, frames as u64)
            .map_err(|_| "Audio encoder could not start")?;
        let mut path = [0u8; sex_pdx::fs::PATH_BYTES];
        let suggestion = format!("mix.{}", format.extension());
        let chosen = files::pick(
            &mut app.window,
            files::Pick::Save,
            suggestion.as_bytes(),
            &mut path,
        )
        .map_err(|_| "Export cancelled")?;
        files::create(&mut app.window, chosen)
            .map_err(|_| "Choose a new filename; destination exists or is unavailable")?;
        studio.set_busy(&mut app.ui, true);
        for t in &mut studio.session.tracks {
            t.reset_effects();
        }
        let result = (|| {
            export_drain(app, &mut encoder, chosen)?;
            let mut block = vec![[0i16; 2]; encoder.frame_capacity()];
            let mut at = 0;
            let mut next_progress = 0;
            while at < frames {
                let n = block.len().min(frames - at);
                moan_core::render_processed(&mut studio.session.tracks, at, &mut block[..n]);
                encoder
                    .push(&block[..n])
                    .map_err(|_| "Audio encoding failed; partial file retained")?;
                export_drain(app, &mut encoder, chosen)?;
                at += n;
                if at >= next_progress || at == frames {
                    studio.message(
                        &mut app.ui,
                        &format!(
                            "Exporting {}: {:.0}%",
                            format.extension(),
                            at as f64 * 100. / frames as f64
                        ),
                    );
                    app.present();
                    next_progress = at + RATE as usize;
                }
            }
            encoder
                .finish()
                .map_err(|_| "Encoder finalization failed; partial file retained")?;
            if !export_drain(app, &mut encoder, chosen)? {
                return Err("Encoder did not finish; partial file retained");
            }
            files::sync(&mut app.window)
                .map_err(|_| "Export sync failed; partial file retained")?;
            sex_pdx::serial_record!(
                "[moan.export] format={} frames={} bytes={} synced=1",
                format.extension(),
                frames,
                encoder.length()
            );
            Ok(())
        })();
        for t in &mut studio.session.tracks {
            t.reset_effects();
        }
        studio.set_busy(&mut app.ui, false);
        result
    }
    fn import(app: &mut App, studio: &mut Studio) -> Result<(), &'static str> {
        let mut path = [0u8; sex_pdx::fs::PATH_BYTES];
        let chosen = files::pick(&mut app.window, files::Pick::Open, b"", &mut path)
            .map_err(|_| "Import cancelled")?;
        let size = files::size(&mut app.window, chosen).map_err(|_| "Cannot read WAV")?;
        if size > moan::MAX_FRAMES as u64 * 4 + 65536 {
            return Err("WAV exceeds one-minute import limit");
        }
        let mut bytes = vec![0u8; size as usize];
        let n = files::read(&mut app.window, chosen, &mut bytes).map_err(|_| "Cannot read WAV")?;
        if n != bytes.len() {
            return Err("Short WAV read");
        }
        studio.session.add(moan::read_wav(&bytes)?)?;
        studio.refresh(&mut app.ui);
        Ok(())
    }
    extern "C" fn main() -> ! {
        let mut app = App::open("Moan", Theme::dark(), Strings::new("en"));
        let mut studio = Studio::build(&mut app.ui);
        app.present();
        sex_pdx::serial_record!(
            "[moan.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        for (id, name) in &studio.buttons {
            let p = app.ui.rect(*id).center();
            sex_pdx::serial_record!(
                "[moan.control] name={} x={} y={}",
                name.replace(' ', "_"),
                p.x as i32,
                p.y as i32
            );
        }
        for (name, id) in [
            ("position", studio.position),
            ("range_start", studio.from),
            ("range_end", studio.to),
        ] {
            let p = app.ui.rect(id).center();
            sex_pdx::serial_record!(
                "[moan.control] name={} x={} y={}",
                name,
                p.x as i32,
                p.y as i32
            );
        }
        let mut sound = Sound::open();
        let mut playing = false;
        let mut paused = false;
        let mut at = 0usize;
        let mut playback_start = 0usize;
        let mut play_ring_start = 0u64;
        let mut lost_start = 0u64;
        let mut recording: Option<Vec<Frame>> = None;
        let mut lost = 0u64;
        let mut block = [[0i16; 2]; 1024];
        loop {
            // Poll the existing window with a short bound while sound is active;
            // toolkit App::next can wait indefinitely when there are no UI timers.
            let responses = if playing || recording.is_some() {
                app.present();
                app.ui.tick(weft::sexos::now());
                let mut out = Vec::new();
                if let Some(e) = app.window.next_within(4) {
                    out.extend(app.ui.handle(weft::sexos::event(e)));
                }
                for _ in 0..64 {
                    let Some(e) = app.window.poll() else { break };
                    out.extend(app.ui.handle(weft::sexos::event(e)));
                }
                out
            } else {
                app.next()
            };
            for r in responses {
                studio.set_busy(&mut app.ui, playing || paused || recording.is_some());
                if r == Response::Close {
                    if let Some(s) = &mut sound {
                        s.listen(false);
                        s.skip();
                    }
                    sex_pdx::serial_record!("[moan.closed]");
                    sex_pdx::task_finish(0);
                }
                match studio.on(&mut app.ui, &r) {
                    Err(e) => studio.message(&mut app.ui, e),
                    Ok(action) => match action {
                        Some(Action::Play) => {
                            if recording.is_some() {
                                studio.message(&mut app.ui, "Stop recording first");
                                continue;
                            }
                            if studio.session.tracks.is_empty() {
                                studio.message(&mut app.ui, "Add audio first");
                                continue;
                            }
                            if let Some(s) = &mut sound {
                                if paused {
                                    s.pause(false);
                                    paused = false;
                                } else {
                                    s.skip();
                                    s.reopen();
                                    s.set_rate(RATE);
                                    at = studio.session.cursor;
                                    playback_start = at;
                                    play_ring_start = s.written_total();
                                    for t in &mut studio.session.tracks {
                                        t.reset_effects();
                                    }
                                }
                                playing = true;
                                studio.message(&mut app.ui, "Playing");
                                sex_pdx::serial_record!(
                                    "[moan.play] tracks={} from={}",
                                    studio.session.tracks.len(),
                                    at
                                );
                            } else {
                                studio.message(&mut app.ui, "No sound device for this program");
                            }
                        }
                        Some(Action::Pause) => {
                            if !playing {
                                continue;
                            }
                            if let Some(s) = &mut sound {
                                s.pause(true);
                                paused = true;
                                playing = false;
                                studio.message(&mut app.ui, "Paused");
                            }
                        }
                        Some(Action::Stop) => {
                            playing = false;
                            paused = false;
                            if let Some(s) = &mut sound {
                                s.pause(false);
                                s.skip();
                                // listen(false) advances the read cursor. Drain the
                                // bounded capture ring first so Stop keeps its tail.
                                if let Some(pcm) = &mut recording {
                                    for _ in 0..(sex_pdx::sound_ring::CAPTURE_FRAMES as usize
                                        / block.len()
                                        + 1)
                                    {
                                        if capture(s, pcm, &mut lost, &mut block) == 0 {
                                            break;
                                        }
                                    }
                                }
                                s.listen(false);
                            }
                            if let Some(pcm) = recording.take() {
                                let n = pcm.len();
                                match studio.session.add(pcm) {
                                    Ok(()) => {
                                        studio.refresh(&mut app.ui);
                                        studio.message(
                                            &mut app.ui,
                                            &format!(
                                                "Recorded {} frames; lost {} microphone frames",
                                                n,
                                                lost.saturating_sub(lost_start)
                                            ),
                                        );
                                        sex_pdx::serial_record!(
                                            "[moan.recorded] frames={} lost={}",
                                            n,
                                            lost.saturating_sub(lost_start)
                                        );
                                    }
                                    Err(e) => studio.message(&mut app.ui, e),
                                }
                            } else {
                                studio.message(&mut app.ui, "Stopped");
                            }
                        }
                        Some(Action::Record) => {
                            if recording.is_some() || studio.session.tracks.len() >= 8 {
                                studio.message(&mut app.ui,"Stop recording first, or start a project with fewer than eight tracks");
                                continue;
                            }
                            if let Some(s) = &mut sound {
                                playing = false;
                                paused = false;
                                s.pause(false);
                                s.skip();
                                s.listen(true);
                                lost = s.lost();
                                lost_start = lost;
                                sex_pdx::serial_record!(
                                    "[moan.record.request] row={}",
                                    sex_pdx::sound_ring::index_of(app.window.target).unwrap_or(0)
                                );
                                recording = Some(Vec::new());
                                studio.message(
                                    &mut app.ui,
                                    "Microphone permission pending; use Stop to finish",
                                );
                            } else {
                                studio.message(&mut app.ui, "No sound ring");
                            }
                        }
                        Some(Action::Import) | Some(Action::Export) => {
                            if playing || paused || recording.is_some() {
                                studio
                                    .message(&mut app.ui, "Stop transport before file operations");
                                continue;
                            }
                            let result = if action == Some(Action::Import) {
                                import(&mut app, &mut studio)
                            } else {
                                export(&mut app, &mut studio)
                            };
                            studio.message(
                                &mut app.ui,
                                match result {
                                    Ok(()) => "File operation complete",
                                    Err(e) => e,
                                },
                            );
                        }
                        Some(_) => {} // Project runtime arrives in the coordinated project patch.
                        None => {
                            sex_pdx::serial_record!(
                                "[moan.project] tracks={} frames={} selected={} clips={}",
                                studio.session.tracks.len(),
                                studio.session.duration(),
                                studio.session.selected,
                                studio
                                    .session
                                    .tracks
                                    .iter()
                                    .map(|t| t.clips.len())
                                    .sum::<usize>()
                            );
                        }
                    },
                }
            }
            if let Some(s) = &mut sound {
                if let Some(pcm) = &mut recording {
                    match s.mic() {
                        sex_pdx::sound_ring::MIC_NONE => {
                            s.listen(false);
                            recording = None;
                            studio.message(&mut app.ui, "No microphone is available");
                        }
                        sex_pdx::sound_ring::MIC_ON => {
                            capture(s, pcm, &mut lost, &mut block);
                            studio.message(
                                &mut app.ui,
                                &format!(
                                    "Recording {:.1} seconds — Stop to keep track",
                                    pcm.len() as f64 / RATE as f64
                                ),
                            );
                            if pcm.len() == moan::MAX_FRAMES {
                                s.listen(false);
                                let data = recording.take().unwrap();
                                let _ = studio.session.add(data);
                                studio.refresh(&mut app.ui);
                                studio.message(
                                    &mut app.ui,
                                    "60-second recording limit reached; track kept",
                                );
                            }
                        }
                        _ => {
                            studio.message(
                                &mut app.ui,
                                &format!(
                                    "Allow microphone in Spindle: volume {} mic on",
                                    sex_pdx::sound_ring::index_of(app.window.target).unwrap_or(0)
                                ),
                            );
                        }
                    }
                }
                if playing {
                    let end = studio.session.duration().saturating_add(RATE as usize * 2);
                    let n = block.len().min(s.room()).min(end.saturating_sub(at));
                    if n > 0 {
                        moan_core::render_processed(
                            &mut studio.session.tracks,
                            at,
                            &mut block[..n],
                        );
                        let written = s.write(&block[..n]);
                        assert_eq!(written, n);
                        at += written;
                    }
                    let heard =
                        playback_start + s.played_total().saturating_sub(play_ring_start) as usize;
                    studio.message(
                        &mut app.ui,
                        &format!(
                            "Playing {:.2} / {:.2} seconds",
                            heard.min(at) as f64 / RATE as f64,
                            end as f64 / RATE as f64
                        ),
                    );
                    if at >= end && !s.playing() {
                        playing = false;
                        s.close();
                        studio.message(&mut app.ui, "Finished playing");
                        sex_pdx::serial_record!("[moan.finished] frames={}", at - playback_start);
                    }
                }
            }
            studio.set_busy(&mut app.ui, playing || paused || recording.is_some());
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(1 << 20, main)
    }
}
#[cfg(not(feature = "sexos"))]
fn main() {
    println!("Moan host checks: cargo test -p moan --no-default-features");
}
