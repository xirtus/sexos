// SPDX-License-Identifier: GPL-3.0-only
//! Editable project Files operations. Save-as uses exclusive CREATE; a partial
//! save never replaces an existing project and never marks the session clean.
use crate::{project, Studio};
use alloc::vec::Vec;
use sex_app::files;
use weft::sexos::App;
/// An ordinary program must explicitly decline Shell's pending close, otherwise
/// its five-second unresponsive-app deadline would bypass the app's dialog.
pub fn decline_close(app: &App) {
    loop {
        match sex_pdx::pdx_call_checked(
            sex_pdx::SLOT_SHELL,
            sex_pdx::app_lifecycle::OP_APP_CLOSE_DECLINED,
            app.window.target as u64,
            app.window.task as u64,
            sex_pdx::app_lifecycle::DECLINE_UNSAVED,
        ) {
            Ok(_) => break,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
            Err(_) => break,
        }
    }
}
/// All callers are stopped for file operations. Dirty projects get a standard
/// discard dialog; a clean project may close immediately without losing edits.
pub fn close_requested(app: &mut App, studio: &mut Studio) {
    if studio.request_close(&mut app.ui) == Some(crate::Action::Quit) {
        let synced = files::sync(&mut app.window).is_ok();
        sex_pdx::serial_record!("[moan.io.partial] synced={}", synced as u8);
        sex_pdx::serial_record!("[moan.closed]");
        sex_pdx::task_finish(0);
    }
    decline_close(app);
}
fn events(app: &mut App, studio: &mut Studio) -> Result<(), &'static str> {
    while let Some(event) = app.window.poll() {
        match event {
            sex_app::Event::Close => {
                close_requested(app, studio);
                return Err(
                    "File operation cancelled; current project kept, partial new output retained",
                );
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
    Ok(())
}
fn open_path(app: &mut App, studio: &mut Studio, path: &[u8]) -> Result<(), &'static str> {
    let size = files::size(&mut app.window, path).map_err(|_| "Cannot inspect project")?;
    if size < project::HEADER as u64 || size > project::MAX_BYTES as u64 {
        return Err("Project is empty or exceeds the 96 MiB limit");
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size as usize)
        .map_err(|_| "Not enough memory to open project")?;
    bytes.resize(size as usize, 0);
    for (index, piece) in bytes.chunks_mut(4096).enumerate() {
        events(app, studio)?;
        if files::read_at(&mut app.window, path, (index * 4096) as u64, piece)
            .map_err(|_| "Cannot read project")?
            != piece.len()
        {
            return Err("Short project read; current project kept");
        }
    }
    events(app, studio)?;
    studio
        .load_project(&mut app.ui, &bytes)
        .map_err(|_| "Invalid or unsupported project; current project kept")?;
    sex_pdx::serial_record!(
        "[moan.project.opened] bytes={} tracks={} clips={} hash={:#x} sources_shared=1 clean=1",
        bytes.len(),
        studio.session.tracks.len(),
        studio
            .session
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        sex_pdx::text_batch::hash(&bytes)
    );
    Ok(())
}
pub fn open(app: &mut App, studio: &mut Studio) -> Result<(), &'static str> {
    let mut path = [0; sex_pdx::fs::PATH_BYTES];
    let picked = files::pick(&mut app.window, files::Pick::Open, b"", &mut path);
    events(app, studio)?;
    let chosen = picked.map_err(|_| "Open project cancelled; current project kept")?;
    open_path(app, studio, chosen)
}
pub fn save(app: &mut App, studio: &mut Studio) -> Result<(), &'static str> {
    let bytes = studio
        .session
        .encode_project()
        .map_err(|_| "Project exceeds format bounds or cannot be allocated")?;
    let mut path = [0; sex_pdx::fs::PATH_BYTES];
    let picked = files::pick(
        &mut app.window,
        files::Pick::Save,
        b"project.moan",
        &mut path,
    );
    events(app, studio)?;
    let chosen = picked.map_err(|_| "Save project cancelled")?;
    files::create(&mut app.window, chosen)
        .map_err(|_| "Choose a new filename; existing projects are preserved")?;
    let result: Result<(), &'static str> = (|| {
        for (index, piece) in bytes.chunks(4096).enumerate() {
            events(app, studio)?;
            let written = files::write_at(&mut app.window, chosen, (index * 4096) as u64, piece)
                .map_err(|_| "Project write failed; partial new file retained")?;
            if written != piece.len() {
                return Err("Disk full; partial new project retained");
            }
        }
        files::sync(&mut app.window).map_err(|_| "Project sync failed; session remains unsaved")?;
        events(app, studio)?;
        studio.session.mark_saved();
        sex_pdx::serial_record!(
            "[moan.project.saved] bytes={} tracks={} clips={} hash={:#x} synced=1",
            bytes.len(),
            studio.session.tracks.len(),
            studio
                .session
                .tracks
                .iter()
                .map(|t| t.clips.len())
                .sum::<usize>(),
            sex_pdx::text_batch::hash(&bytes)
        );
        Ok(())
    })();
    if result.is_err() {
        let synced = files::sync(&mut app.window).is_ok();
        sex_pdx::serial_record!("[moan.project.partial] synced={}", synced as u8);
    }
    result
}
/// The Shell can launch a held project route. Loading it never widens authority.
pub fn startup(app: &mut App, studio: &mut Studio) {
    let mut args = [0; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut app.window, &mut args);
    let path = given.trim_ascii();
    if path.is_empty() {
        return;
    }
    match open_path(app, studio, path) {
        Ok(()) => studio.message(&mut app.ui, "Editable project opened"),
        Err(error) => studio.message(&mut app.ui, error),
    }
}
/// Serial receipts describe the actual current scrolled/scaled content rectangle
/// for guest pointer gates; the document data itself is not logged.
pub fn log(app: &mut App, studio: &Studio) {
    app.ui.layout();
    app.present();
    let rect = app.ui.rect(studio.waveform);
    let scale = app.ui.theme().scale;
    sex_pdx::serial_record!(
        "[moan.timeline] x={} y={} w={} h={} start={} span={}",
        (rect.x * scale) as i32,
        (rect.y * scale) as i32,
        (rect.w * scale) as i32,
        (rect.h * scale) as i32,
        studio.view.start,
        studio.view.span
    );
    for (id, name) in &studio.buttons {
        let rect = app.ui.rect(*id);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let p = rect.center();
        sex_pdx::serial_record!(
            "[moan.control] name={} x={} y={}",
            name.replace(' ', "_"),
            (p.x * scale) as i32,
            (p.y * scale) as i32
        );
    }
    for (name, id) in [
        ("position", studio.position),
        ("range_start", studio.from),
        ("range_end", studio.to),
    ] {
        let rect = app.ui.rect(id);
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let p = rect.center();
        sex_pdx::serial_record!(
            "[moan.control] name={} x={} y={}",
            name,
            (p.x * scale) as i32,
            (p.y * scale) as i32
        );
    }
    if let Some(id) = app.ui.find_text("Discard") {
        let rect = app.ui.rect(id);
        let p = rect.center();
        sex_pdx::serial_record!(
            "[moan.dialog] action=Discard x={} y={} w={} h={}",
            (p.x * scale) as i32,
            (p.y * scale) as i32,
            (rect.w * scale) as i32,
            (rect.h * scale) as i32
        );
    }
    let piano = app.ui.rect(studio.piano);
    sex_pdx::serial_record!(
        "[moan.piano] x={} y={} w={} h={}",
        (piano.x * scale) as i32,
        (piano.y * scale) as i32,
        (piano.w * scale) as i32,
        (piano.h * scale) as i32
    );
    for (name, id) in [
        ("step_gate", studio.gate),
        ("step_velocity", studio.step_velocity),
    ] {
        let rect = app.ui.rect(id);
        sex_pdx::serial_record!(
            "[moan.slider] name={} x={} y={} w={} h={} value={}",
            name,
            (rect.x * scale) as i32,
            (rect.y * scale) as i32,
            (rect.w * scale) as i32,
            (rect.h * scale) as i32,
            app.ui.value(id) as u32
        );
    }
    let steps = app.ui.rect(studio.steps);
    sex_pdx::serial_record!(
        "[moan.steps] x={} y={} w={} h={} gate={} velocity={}",
        (steps.x * scale) as i32,
        (steps.y * scale) as i32,
        (steps.w * scale) as i32,
        (steps.h * scale) as i32,
        studio.note_gate,
        studio.note_velocity
    );
    if let Some(pattern) = studio
        .session
        .midi_clip()
        .and_then(moan_core::Clip::midi_pattern)
    {
        sex_pdx::serial_record!(
            "[moan.midi] track={} instrument={} notes={} frames={} bpm={} hash={:#x}",
            studio.session.selected,
            pattern.instrument as u8,
            pattern.notes.len(),
            pattern.frames,
            pattern.bpm,
            pattern.fingerprint()
        );
    }
    for (track, id) in studio
        .meter_values
        .iter()
        .enumerate()
        .take(studio.session.tracks.len())
    {
        let rect = app.ui.rect(*id);
        sex_pdx::serial_record!(
            "[moan.meter] track={} x={} y={} w={} h={} level={}",
            track,
            (rect.x * scale) as i32,
            (rect.y * scale) as i32,
            (rect.w * scale) as i32,
            (rect.h * scale) as i32,
            (app.ui.value(*id) * 10000.0) as u32
        );
    }
    let status = app.ui.rect(studio.plugin_status);
    sex_pdx::serial_record!(
        "[moan.plugin.status] state={:?} x={} y={} w={} h={} format={} text={}",
        studio.plugin_state,
        (status.x * scale) as i32,
        (status.y * scale) as i32,
        (status.w * scale) as i32,
        (status.h * scale) as i32,
        studio.plugin_format,
        app.ui.text_of(studio.plugin_status)
    );
    for (name, id) in [
        ("master_level", studio.master_level),
        ("master_meter", studio.master_meter),
    ] {
        let rect = app.ui.rect(id);
        sex_pdx::serial_record!(
            "[moan.mixer.control] name={} x={} y={} w={} h={} value={}",
            name,
            (rect.x * scale) as i32,
            (rect.y * scale) as i32,
            (rect.w * scale) as i32,
            (rect.h * scale) as i32,
            (app.ui.value(id) * 100.0) as i32
        );
    }
    let (w, h) = app.ui.size();
    sex_pdx::serial_record!("[moan.viewport] w={} h={} frame={}", w, h, app.frames);
    sex_pdx::serial_record!(
        "[moan.selection] track={} cursor={} from={} to={} dirty={}",
        studio.session.selected,
        studio.session.cursor,
        studio.session.from,
        studio.session.to,
        studio.session.is_dirty() as u8
    );
}
