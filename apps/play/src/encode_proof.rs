//! Explicit synthetic encoder qualification, not desktop recording.
use alloc::vec;
use sex_app::{files, Event, Window};
use sex_media::recording::{Config, Drain, Encoder};

fn drain(window: &mut Window, encoder: &mut Encoder, path: &[u8]) -> Result<bool, &'static [u8]> {
    loop {
        if let Some(Event::Close) = window.poll() {
            // No Files request is in flight between drain turns. End the run
            // with its named partial retained; do not consume Close and leave
            // an apparently closed proof window alive.
            sex_pdx::serial_record!("[play.encode.cancelled] partial_retained=1");
            sex_pdx::task_finish(0);
        }
        match encoder
            .output()
            .map_err(|_| &b"Encoder output failed."[..])?
        {
            Drain::NeedFrame => return Ok(false),
            Drain::End => return Ok(true),
            Drain::Write(write) => {
                let (token, offset, length) = (write.token, write.offset, write.bytes.len());
                let wrote = files::write_at(window, path, offset, write.bytes)
                    .map_err(|_| &b"Cannot write encoder proof."[..])?;
                if wrote != length {
                    return Err(b"Short encoder proof write.");
                }
                encoder
                    .acknowledge(token, wrote)
                    .map_err(|_| &b"Encoder acknowledgement failed."[..])?;
            }
        }
    }
}

pub fn run(window: &mut Window, path: &[u8]) -> Result<(), &'static [u8]> {
    if path.is_empty()
        || path.contains(&b'/')
        || path.contains(&b'@')
        || path.contains(&0)
        || !path.ends_with(b".mkv")
        || path.len() > 200
    {
        return Err(b"Usage: run play --encode-proof proof.mkv");
    }
    // Friendly early refusal; the exclusive CREATE below is the actual fence.
    match files::size(window, path) {
        Ok(_) => return Err(b"Encoder proof file already exists."),
        Err(sex_pdx::fs::NOT_FOUND) => {}
        Err(_) => return Err(b"Cannot inspect encoder proof destination."),
    }
    let mut encoder = Encoder::new(Config {
        width: 65,
        height: 49,
        fps: 5,
    })
    .map_err(|_| &b"This build cannot create the encoder proof."[..])?;
    // CREATE atomically refuses an existing final name. No replacement rename
    // or deletion can clobber a concurrently created file. Failed output stays
    // named and partial; only successful sync is reported as saved.
    files::create(window, path)
        .map_err(|_| &b"Cannot create encoder proof; destination may exist."[..])?;
    let result = (|| {
        let mut pixels = vec![0u32; 65 * 49];
        drain(window, &mut encoder, path)?;
        for (frame, timestamp) in [0, 200, 600, 800].into_iter().enumerate() {
            for y in 0..49 {
                for x in 0..65 {
                    let r = ((x + frame * 17) % 256) as u32;
                    let g = ((y * 3 + frame * 7) % 256) as u32;
                    let b = (((x ^ y) + frame * 33) % 256) as u32;
                    pixels[y * 65 + x] = (r << 16) | (g << 8) | b;
                }
            }
            encoder
                .frame(&pixels, timestamp)
                .map_err(|_| &b"Encoder proof frame failed."[..])?;
            pixels.fill(0x00ff_00ff); // Submitted frames must own their pixels.
            drain(window, &mut encoder, path)?;
        }
        encoder
            .finish()
            .map_err(|_| &b"Encoder proof finish failed."[..])?;
        if !drain(window, &mut encoder, path)? {
            return Err(&b"Encoder proof did not finish."[..]);
        }
        files::sync(window).map_err(|_| &b"Cannot sync encoder proof."[..])?;
        sex_pdx::serial_record!("[play.encode.proof] synthetic=1 frames={} width=65 height=49 fps=5 pts_ms=0,200,600,800 bytes={} synced=1", encoder.frames(), encoder.length());
        Ok(())
    })();
    result
}
