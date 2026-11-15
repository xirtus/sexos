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
        meter: &mut serenade_core::LevelMeter,
    ) -> usize {
        let n = sound.hear(block);
        for f in &block[..n] {
            meter.process_frame(&[f[0] as f32 / 32768.0, f[1] as f32 / 32768.0]);
        }
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
        studio: &mut Studio,
        encoder: &mut moan_export::Encoder,
        path: &[u8],
    ) -> Result<bool, &'static str> {
        loop {
            while let Some(event) = app.window.poll() {
                match event {
                    sex_app::Event::Close => {
                        sex_pdx::serial_record!("[moan.export.cancelled] partial_retained=1");
                        moan::project_io::close_requested(app, studio);
                        return Err("Export cancelled; partial new file retained");
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
        sex_pdx::serial_record!(
            "[moan.export.started] format={} frames={}",
            format.extension(),
            frames
        );
        let result = (|| {
            export_drain(app, studio, &mut encoder, chosen)?;
            let mut block = vec![[0i16; 2]; encoder.frame_capacity()];
            let mut at = 0;
            let mut next_progress = 0;
            while at < frames {
                let n = block.len().min(frames - at);
                studio.session.render_processed(at, &mut block[..n]);
                encoder
                    .push(&block[..n])
                    .map_err(|_| "Audio encoding failed; partial file retained")?;
                export_drain(app, studio, &mut encoder, chosen)?;
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
            if !export_drain(app, studio, &mut encoder, chosen)? {
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
        if result.is_err() {
            let synced = files::sync(&mut app.window).is_ok();
            sex_pdx::serial_record!("[moan.export.partial] synced={}", synced as u8);
        }
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
    fn choose_plugin_path(app: &mut App, folder: bool) -> Result<Vec<u8>, &'static str> {
        let mut buffer = [0u8; sex_pdx::fs::PATH_BYTES];
        let picked = files::pick(
            &mut app.window,
            if folder {
                files::Pick::Folder
            } else {
                files::Pick::Open
            },
            b"",
            &mut buffer,
        )
        .map_err(|_| "Plugin selection cancelled")?;
        let mut path = Vec::new();
        path.try_reserve_exact(picked.len())
            .map_err(|_| "Not enough memory for plugin selection")?;
        path.extend_from_slice(picked);
        Ok(path)
    }
    extern "C" fn main() -> ! {
        let mut app = App::open(
            "Moan",
            Theme::dark().with_density(weft::Density::Compact),
            Strings::new("en"),
        );
        let mut studio = Studio::build(&mut app.ui);
        moan::project_io::startup(&mut app, &mut studio);
        app.present();
        moan::project_io::log(&mut app, &studio);
        sex_pdx::serial_record!(
            "[moan.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        let mut sound = Sound::open();
        let mut plugin = moan::plugins::guest::Render::default();
        let mut plugin_path = Vec::new();
        let mut render_folder = Vec::new();
        let mut render_at = 0usize;
        let mut pending_plugin_close = false;
        let mut playing = false;
        let mut paused = false;
        let mut at = 0usize;
        let mut playback_start = 0usize;
        let mut play_ring_start = 0u64;
        let mut lost_start = 0u64;
        let mut recording: Option<Vec<Frame>> = None;
        let mut lost = 0u64;
        let mut block = [[0i16; 2]; 1024];
        let mut meters: [serenade_core::LevelMeter; moan::MAX_TRACKS] =
            core::array::from_fn(|_| serenade_core::LevelMeter::new(2, RATE, 800, 18.0));
        let mut master_meter = serenade_core::LevelMeter::new(2, RATE, 800, 18.0);
        let mut meter_due = 0u64;
        let mut status_due = 0u64;
        let mut prefill = false;
        // Twice the maximum queued ring: rendering a new batch cannot overwrite
        // any not-yet-metered consumed batch. All storage predates playback.
        let mut history =
            moan::meter_history::History::new(sex_pdx::sound_ring::FRAMES as usize * 2);
        loop {
            if plugin.poll(weft::sexos::now()) {
                sex_pdx::serial_record!(
                    "[moan.plugin.state] status={:?} retired={}",
                    plugin.status(),
                    plugin.retired() as u8
                );
                studio.plugin_status(&mut app.ui, plugin.status());
            }
            if let Some(audio) = plugin.take_result() {
                let frames = audio.frames.len();
                let clipped = audio.clipped_samples;
                studio.session.cursor = render_at;
                match studio.session.add(audio.frames) {
                    Ok(()) => {
                        studio.refresh(&mut app.ui);
                        studio.message(&mut app.ui,&format!("Offline render added as a new track; original kept · {} clipped samples",clipped));
                        sex_pdx::serial_record!(
                            "[moan.plugin.imported] tracks={} frames={} clipped={} format={:?}",
                            studio.session.tracks.len(),
                            frames,
                            clipped,
                            audio.format
                        );
                    }
                    Err(error) => studio.message(&mut app.ui, error),
                }
                studio.plugin_status(&mut app.ui, plugin.status());
            }
            if pending_plugin_close && plugin.retired() {
                pending_plugin_close = false;
                if studio.request_close_with_pending(&mut app.ui, recording.is_some())
                    == Some(Action::Quit)
                {
                    if let Some(sound) = &mut sound {
                        sound.listen(false);
                        sound.skip();
                        sound.close();
                    }
                    sex_pdx::serial_record!("[moan.closed]");
                    sex_pdx::task_finish(0);
                }
                moan::project_io::decline_close(&app);
            }
            studio.set_activity(
                &mut app.ui,
                playing || paused || recording.is_some() || plugin.busy(),
                (playing || paused) && recording.is_none() && !plugin.busy(),
            );
            // Poll the existing window with a short bound while sound is active;
            // toolkit App::next can wait indefinitely when there are no UI timers.
            let responses = if playing || recording.is_some() || plugin.busy() {
                app.present();
                app.ui.tick(weft::sexos::now());
                let mut out = Vec::new();
                if let Some(e) =
                    app.window
                        .next_within(if plugin.busy() && !playing && recording.is_none() {
                            20
                        } else {
                            4
                        })
                {
                    out.extend(studio.handle_event(&mut app.ui, weft::sexos::event(e)));
                }
                for _ in 0..64 {
                    let Some(e) = app.window.poll() else { break };
                    out.extend(studio.handle_event(&mut app.ui, weft::sexos::event(e)));
                }
                out
            } else {
                studio.next(&mut app)
            };
            let had_responses = !responses.is_empty();
            for r in responses {
                studio.set_activity(
                    &mut app.ui,
                    playing || paused || recording.is_some() || plugin.busy(),
                    (playing || paused) && recording.is_none() && !plugin.busy(),
                );
                let outcome = if r == Response::Close {
                    if plugin.busy() && !plugin.retired() {
                        plugin.cancel();
                        pending_plugin_close = true;
                        studio.plugin_status(&mut app.ui, plugin.status());
                        studio.message(&mut app.ui, "Cancelling offline render before closing");
                        moan::project_io::decline_close(&app);
                        continue;
                    }
                    if plugin.busy() {
                        plugin.cancel();
                        studio.plugin_status(&mut app.ui, plugin.status());
                    }
                    let action =
                        studio.request_close_with_pending(&mut app.ui, recording.is_some());
                    if action.is_none() {
                        moan::project_io::decline_close(&app);
                    }
                    Ok(action)
                } else {
                    studio.on(&mut app.ui, &r)
                };
                match outcome {
                    Err(e) => studio.message(&mut app.ui, e),
                    Ok(action) => match action {
                        Some(Action::ChoosePlugin) | Some(Action::ChooseRenderFolder) => {
                            if playing || paused || recording.is_some() || plugin.busy() {
                                studio.message(
                                    &mut app.ui,
                                    "Stop transport and offline render first",
                                );
                                continue;
                            }
                            let folder = action == Some(Action::ChooseRenderFolder);
                            match choose_plugin_path(&mut app, folder) {
                                Ok(path) => {
                                    studio.plugin_selection(&mut app.ui, folder, &path);
                                    if folder {
                                        render_folder = path;
                                    } else {
                                        plugin_path = path;
                                    }
                                }
                                Err(error) => studio.message(&mut app.ui, error),
                            }
                        }
                        Some(Action::RenderPlugin) => {
                            if plugin.status() == moan::plugins::Status::WaitingForMemory {
                                plugin.retry_result();
                                studio.plugin_status(&mut app.ui, plugin.status());
                                continue;
                            }
                            if playing || paused || recording.is_some() {
                                studio.message(
                                    &mut app.ui,
                                    "Stop transport before offline rendering",
                                );
                                continue;
                            }
                            if plugin_path.is_empty() || render_folder.is_empty() {
                                studio.message(
                                    &mut app.ui,
                                    "Choose a plugin and a render folder first",
                                );
                                continue;
                            }
                            let result = (|| {
                                let (start, pcm) = studio.session.offline_region()?;
                                let format = if studio.plugin_format == 0 {
                                    moan_plugin::job::Format::Clap
                                } else {
                                    moan_plugin::job::Format::Vst3
                                };
                                plugin
                                    .start(
                                        &plugin_path,
                                        &render_folder,
                                        &pcm,
                                        format,
                                        &[],
                                        weft::sexos::now(),
                                    )
                                    .map_err(|error| error.message())?;
                                render_at = start;
                                Ok::<(), &'static str>(())
                            })();
                            match result {
                                Ok(()) => {
                                    studio.plugin_status(&mut app.ui, plugin.status());
                                    studio.set_busy(&mut app.ui, true);
                                    sex_pdx::serial_record!(
                                        "[moan.plugin.started] format={} input_region_start={}",
                                        studio.plugin_format,
                                        render_at
                                    );
                                }
                                Err(error) => studio.message(&mut app.ui, error),
                            }
                        }
                        Some(Action::CancelPlugin) => {
                            plugin.cancel();
                            studio.plugin_status(&mut app.ui, plugin.status());
                        }
                        Some(Action::Play) => {
                            if plugin.busy() {
                                studio.message(
                                    &mut app.ui,
                                    "Wait for the offline render before playback",
                                );
                                continue;
                            }
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
                                    s.pause(true);
                                    prefill = true;
                                    s.set_rate(RATE);
                                    at = studio.session.cursor;
                                    playback_start = at;
                                    play_ring_start = s.written_total();
                                    history.reset(at);
                                    status_due = 0;
                                    for t in &mut studio.session.tracks {
                                        t.reset_effects();
                                    }
                                    for meter in &mut meters {
                                        meter.reset();
                                    }
                                    master_meter.reset();
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
                            prefill = false;
                            studio.transport_cursor(&mut app.ui, None);
                            for meter in &mut meters {
                                meter.reset();
                            }
                            master_meter.reset();
                            playing = false;
                            paused = false;
                            if let Some(s) = &mut sound {
                                s.pause(false);
                                s.skip();
                                s.close();
                                // listen(false) advances the read cursor. Drain the
                                // bounded capture ring first so Stop keeps its tail.
                                if let Some(pcm) = &mut recording {
                                    for _ in 0..(sex_pdx::sound_ring::CAPTURE_FRAMES as usize
                                        / block.len()
                                        + 1)
                                    {
                                        if capture(
                                            s,
                                            pcm,
                                            &mut lost,
                                            &mut block,
                                            &mut meters[studio
                                                .session
                                                .tracks
                                                .len()
                                                .min(moan::MAX_TRACKS - 1)],
                                        ) == 0
                                        {
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
                                sex_pdx::serial_record!("[moan.stopped] queued_discarded=1");
                            }
                        }
                        Some(Action::Record) => {
                            if recording.is_some() || studio.session.tracks.len() >= 8 {
                                studio.message(&mut app.ui,"Stop recording first, or start a project with fewer than eight tracks");
                                continue;
                            }
                            let mut pcm = Vec::new();
                            if pcm.try_reserve_exact(moan::MAX_FRAMES).is_err() {
                                studio.message(&mut app.ui, "Not enough memory to record");
                                continue;
                            }
                            if let Some(s) = &mut sound {
                                studio.transport_cursor(&mut app.ui, None);
                                playing = false;
                                paused = false;
                                s.pause(false);
                                s.skip();
                                s.listen(true);
                                for meter in &mut meters {
                                    meter.reset();
                                }
                                master_meter.reset();
                                lost = s.lost();
                                lost_start = lost;
                                sex_pdx::serial_record!(
                                    "[moan.record.request] row={}",
                                    sex_pdx::sound_ring::index_of(app.window.target).unwrap_or(0)
                                );
                                recording = Some(pcm);
                                studio.message(
                                    &mut app.ui,
                                    "Microphone permission pending; use Stop to finish",
                                );
                            } else {
                                studio.message(&mut app.ui, "No sound ring");
                            }
                        }
                        Some(Action::Import) | Some(Action::Export) => {
                            if playing || paused || recording.is_some() || plugin.busy() {
                                studio
                                    .message(&mut app.ui, "Stop transport and finish offline rendering before file operations");
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
                        Some(Action::OpenProject) | Some(Action::SaveProject) => {
                            if playing || paused || recording.is_some() || plugin.busy() {
                                studio.message(
                                    &mut app.ui,
                                    "Stop transport and finish offline rendering before project operations",
                                );
                                continue;
                            }
                            let result = if action == Some(Action::OpenProject) {
                                moan::project_io::open(&mut app, &mut studio)
                            } else {
                                moan::project_io::save(&mut app, &mut studio)
                            };
                            studio.message(
                                &mut app.ui,
                                match result {
                                    Ok(()) => "Editable project operation complete",
                                    Err(error) => error,
                                },
                            );
                        }
                        Some(Action::NewProject) => {
                            if plugin.busy() {
                                studio.message(
                                    &mut app.ui,
                                    "Finish or cancel offline rendering first",
                                );
                                continue;
                            }
                            studio.new_project(&mut app.ui);
                            studio.message(&mut app.ui, "New editable project");
                            sex_pdx::serial_record!("[moan.project.new] tracks=0 clean=1");
                        }
                        Some(Action::Quit) => {
                            if plugin.busy() && !plugin.retired() {
                                plugin.cancel();
                                pending_plugin_close = true;
                                studio.plugin_status(&mut app.ui, plugin.status());
                                moan::project_io::decline_close(&app);
                                continue;
                            }
                            if let Some(s) = &mut sound {
                                s.listen(false);
                                s.skip();
                            }
                            sex_pdx::serial_record!("[moan.closed]");
                            sex_pdx::task_finish(0);
                        }
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
            if had_responses {
                moan::project_io::log(&mut app, &studio);
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
                            for _ in 0..64 {
                                if capture(
                                    s,
                                    pcm,
                                    &mut lost,
                                    &mut block,
                                    &mut meters
                                        [studio.session.tracks.len().min(moan::MAX_TRACKS - 1)],
                                ) == 0
                                {
                                    break;
                                }
                            }
                            if weft::sexos::now() >= status_due {
                                studio.message(
                                    &mut app.ui,
                                    &format!(
                                        "Recording {:.1} seconds — Stop to keep track",
                                        pcm.len() as f64 / RATE as f64
                                    ),
                                );
                                status_due = weft::sexos::now().saturating_add(200);
                            }
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
                    let heard =
                        playback_start + s.played_total().saturating_sub(play_ring_start) as usize;
                    let lost_meter_frames =
                        history.advance_master(heard, at, &mut meters, &mut master_meter);
                    if lost_meter_frames > 0 {
                        sex_pdx::serial_record!(
                            "[moan.meter.overrun] frames={}",
                            lost_meter_frames
                        );
                    }
                    let mut budget = moan::meter_history::fill_budget(
                        s.room(),
                        end.saturating_sub(at),
                        block.len(),
                    );
                    while budget > 0 {
                        let n = block.len().min(budget);
                        let start = at;
                        let mut track_offsets = [0usize; moan::MAX_TRACKS];
                        studio.session.render_processed_observed(
                            at,
                            &mut block[..n],
                            |track, frame| {
                                history.write(track, start + track_offsets[track], frame);
                                track_offsets[track] += 1;
                            },
                        );
                        for (index, frame) in block[..n].iter().enumerate() {
                            history.write(moan::MAX_TRACKS, start + index, *frame);
                        }
                        let written = s.write(&block[..n]);
                        assert_eq!(written, n);
                        at += written;
                        budget -= written;
                    }
                    if prefill {
                        sex_pdx::serial_record!(
                            "[moan.prefilled] frames={} from={}",
                            s.pending(),
                            playback_start
                        );
                        s.pause(false);
                        prefill = false;
                    }
                    if weft::sexos::now() >= status_due {
                        studio.transport_cursor(&mut app.ui, Some(heard.min(at)));
                        studio.message(
                            &mut app.ui,
                            &format!(
                                "Playing {:.2} / {:.2} seconds",
                                heard.min(at) as f64 / RATE as f64,
                                end as f64 / RATE as f64
                            ),
                        );
                        status_due = weft::sexos::now().saturating_add(200);
                    }
                    if at >= end && !s.playing() {
                        playing = false;
                        s.close();
                        studio.transport_cursor(&mut app.ui, None);
                        studio.message(&mut app.ui, "Finished playing");
                        sex_pdx::serial_record!("[moan.finished] frames={}", at - playback_start);
                    }
                }
            }
            let now = weft::sexos::now();
            if had_responses || now >= meter_due {
                studio.publish_meters(&mut app.ui, &mut meters);
                studio.publish_master(&mut app.ui, &mut master_meter);
                meter_due = now.saturating_add(100);
            }
            studio.set_activity(
                &mut app.ui,
                playing || paused || recording.is_some() || plugin.busy(),
                (playing || paused) && recording.is_none() && !plugin.busy(),
            );
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        if sex_app::heap::set_limit(256 * 1024 * 1024).is_err() {
            sex_pdx::task_finish(1);
        }
        sex_pdx::serial_record!("[moan.heap.limit] bytes=268435456");
        sex_app::with_stack(1 << 20, main)
    }
}
#[cfg(not(feature = "sexos"))]
fn main() {
    println!("Moan host checks: cargo test -p moan --no-default-features");
}
