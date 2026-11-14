// SPDX-License-Identifier: GPL-3.0-only
//! Capability-bound I/O and native event loop, intentionally outside the UI model.
use crate::{Action, Document, Kind, Sample, TEXT_LIMIT};
use alloc::{format, string::String, vec};
use sex_app::{files, net};
use weft::{sexos::App, Response, Strings, Theme};
fn read(app: &mut App, path: &str, limit: usize) -> Result<alloc::vec::Vec<u8>, &'static str> {
    let size = files::size(&mut app.window, path.as_bytes()).map_err(|_| "Cannot inspect file")?;
    if size > limit as u64 {
        return Err("File exceeds sample's size limit");
    }
    let mut bytes = vec![0; size as usize];
    let got = files::read(&mut app.window, path.as_bytes(), &mut bytes)
        .map_err(|_| "Cannot read file")?;
    if got != bytes.len() {
        return Err("File changed or read was short");
    }
    Ok(bytes)
}
fn image(app: &mut App, sample: &Sample, bytes: &[u8]) -> Result<(), &'static str> {
    let img = sex_image::decode(
        bytes,
        &sex_image::Limits {
            max_width: 4096,
            max_height: 4096,
            max_pixels: 4_000_000,
            max_input_bytes: 1 << 20,
        },
    )
    .map_err(|_| "Unsupported, damaged or oversized picture")?
    .fit_within(480, 280)
    .map_err(|_| "Cannot fit picture")?;
    sex_pdx::serial_record!("[leather.image] width={} height={}", img.width, img.height);
    app.ui
        .set_image(sample.image, img.width, img.height, img.pixels);
    Ok(())
}
pub fn run(kind: Kind) -> ! {
    let mut app = App::open(kind.title(), Theme::light(), Strings::new("en"));
    let mut sample = Sample::build(&mut app.ui, kind);
    let mut args = [0; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut app.window, &mut args);
    if let Ok(text) = core::str::from_utf8(given) {
        if !text.trim().is_empty() {
            app.ui.set_text(sample.path, text.trim());
        }
    }
    if kind == Kind::Lantern {
        if let Err(e) = image(&mut app, &sample, include_bytes!("../assets/swatches.png")) {
            app.ui.set_text(sample.status, e);
        }
    }
    app.present();
    sex_pdx::serial_record!(
        "[leather.ready] app={} target={} task={} frames={}",
        kind.name(),
        app.window.target,
        app.window.task,
        app.frames
    );
    for (name, id) in [
        ("primary", sample.primary),
        ("load", sample.secondary),
        ("previous", sample.previous),
        ("next", sample.next),
        ("add", sample.add),
        ("path", sample.path),
        ("content", sample.content),
    ] {
        let p = app.ui.rect(id).center();
        let scale = app.ui.theme().scale;
        sex_pdx::serial_record!(
            "[leather.control] app={} name={} x={} y={}",
            kind.name(),
            name,
            (p.x * scale) as i32,
            (p.y * scale) as i32
        );
    }
    loop {
        for response in app.next() {
            if response == Response::Close {
                sex_pdx::task_finish(0);
            }
            let Some(action) = sample.on(&mut app.ui, &response) else {
                if response == Response::Clicked(sample.primary) && kind == Kind::Hello {
                    sex_pdx::serial_record!("[leather.greet] ok=1");
                }
                continue;
            };
            app.ui.set_text(sample.status, "Working...");
            app.present();
            let result: Result<String, &str> = match action {
                Action::Save(path, text) => (|| {
                    files::write(&mut app.window, path.as_bytes(), text.as_bytes())
                        .map_err(|_| "Cannot save file")?;
                    files::sync(&mut app.window).map_err(|_| "Cannot sync file")?;
                    sex_pdx::serial_record!("[leather.saved] bytes={} synced=1", text.len());
                    Ok(format!("Saved {} bytes", text.len()))
                })(),
                Action::Load(path) => (|| {
                    let data = read(&mut app, &path, TEXT_LIMIT)?;
                    let document = Document::parse(&data)?;
                    sample.loaded(&mut app.ui, document);
                    sex_pdx::serial_record!(
                        "[leather.loaded] bytes={} lines={} hash={:#x}",
                        data.len(),
                        sample.document.lines.len(),
                        sex_pdx::text_batch::hash(&data)
                    );
                    Ok(format!("Loaded {} bytes", data.len()))
                })(),
                Action::Image(path) => (|| {
                    let bytes = read(&mut app, &path, 1 << 20)?;
                    image(&mut app, &sample, &bytes)?;
                    Ok("Picture opened".into())
                })(),
                Action::Fetch(url) => (|| {
                    if !(url.starts_with("http://") || url.starts_with("https://")) {
                        return Err("Enter an http:// or https:// URL");
                    }
                    let mut bytes = vec![0; 16384];
                    let (status, n) = net::get(&mut app.window, url.as_bytes(), &mut bytes)
                        .map_err(|e| {
                            core::str::from_utf8(net::message(e)).unwrap_or("Fetch failed")
                        })?;
                    sex_pdx::serial_record!(
                        "[leather.fetched] status={} bytes={} hash={:#x}",
                        status,
                        n,
                        sex_pdx::text_batch::hash(&bytes[..n])
                    );
                    Ok(format!(
                        "HTTP {status}\n{}",
                        String::from_utf8_lossy(&bytes[..n])
                    ))
                })(),
            };
            match result {
                Ok(text) => app.ui.set_text(sample.status, &text),
                Err(error) => {
                    app.ui.set_text(sample.status, error);
                    sex_pdx::serial_record!("[leather.error] app={} what={}", kind.name(), error);
                }
            }
        }
    }
}
