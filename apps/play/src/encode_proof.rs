//! Explicit synthetic encoder qualification, not desktop recording.
use alloc::vec;
use sex_app::{files, Event, Window};
use sex_media::recording::{Config, Drain, Encoder};

fn drain(window: &mut Window, encoder: &mut Encoder, path: &[u8]) -> Result<bool, &'static [u8]> {
    loop {
        if let Some(Event::Close) = window.poll() {
            return Err(b"Encoding cancelled.");
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
    // Refuse overwrite. Existing names and failures are never removed here.
    match files::size(window, path) {
        Ok(_) => return Err(b"Encoder proof file already exists."),
        Err(sex_pdx::fs::NOT_FOUND) => {}
        Err(_) => return Err(b"Cannot inspect encoder proof destination."),
    }
    let mut temp = path.to_vec();
    temp.extend_from_slice(b".part");
    match files::size(window, &temp) {
        Ok(_) => return Err(b"Encoder proof temporary file already exists."),
        Err(sex_pdx::fs::NOT_FOUND) => {}
        Err(_) => return Err(b"Cannot inspect encoder proof temporary file."),
    }
    let mut encoder = Encoder::new(Config {
        width: 65,
        height: 49,
        fps: 5,
    })
    .map_err(|_| &b"This build cannot create the encoder proof."[..])?;
    files::create(window, &temp).map_err(|_| &b"Cannot create encoder proof."[..])?;
    let result = (|| {
        let mut pixels = vec![0u32; 65 * 49];
        drain(window, &mut encoder, &temp)?;
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
            drain(window, &mut encoder, &temp)?;
        }
        encoder
            .finish()
            .map_err(|_| &b"Encoder proof finish failed."[..])?;
        if !drain(window, &mut encoder, &temp)? {
            return Err(&b"Encoder proof did not finish."[..]);
        }
        files::rename(window, &temp, path).map_err(|_| &b"Cannot rename encoder proof."[..])?;
        files::sync(window).map_err(|_| &b"Cannot sync encoder proof."[..])?;
        sex_pdx::serial_record!("[play.encode.proof] synthetic=1 frames={} width=65 height=49 fps=5 pts_ms=0,200,600,800 bytes={} synced=1", encoder.frames(), encoder.length());
        Ok(())
    })();
    if result.is_err() {
        let _ = files::remove(window, &temp);
    }
    result
}
