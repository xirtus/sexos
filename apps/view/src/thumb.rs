// SPDX-License-Identifier: GPL-3.0-only
//! `gaze --thumb @IN @OUT SIZE`: a small picture of a photo for Loom, with no window.
//!
//! The same contract as Vellum's (`apps/pdf/src/thumb.rs`): the photo is drawn upright
//! (EXIF orientation applied) to fit inside SIZE by SIZE, never larger than it is, and
//! written as
//!
//! ```text
//! width: u16 LE, height: u16 LE, then width * height * 3 bytes of RGB, rows top to bottom
//! ```
//!
//! with transparent parts laid on white. The output is written whole or not at all,
//! and the run ends with status 0 only when it was written. Gaze reads everything
//! gaze-codecs knows: camera RAW files give their embedded preview.
use alloc::vec::Vec;
use sex_app::files::direct;
use sex_image::{Image, Limits};

/// The largest SIZE accepted: the header holds 16-bit sides.
const MAX_SIZE: usize = 4096;
/// The largest input read: camera RAW files run to tens of megabytes.
const MAX_INPUT: u64 = 96 << 20;
const LIMITS: Limits = Limits {
    max_width: 16384,
    max_height: 16384,
    max_pixels: 50_000_000,
    max_input_bytes: MAX_INPUT as usize,
};

/// `Some(status)` when the words start with `--thumb`: the run was a thumbnail and
/// ends with that status. `None` for an ordinary run.
pub fn run(words: &[Vec<u8>]) -> Option<u32> {
    if words.first().map(Vec::as_slice) != Some(b"--thumb") {
        return None;
    }
    let started = sex_pdx::get_ticks();
    let status = match make(words) {
        Ok((w, h)) => {
            sex_pdx::serial_record!("[gaze.thumb] w={} h={} ms={}", w, h, sex_pdx::get_ticks().saturating_sub(started));
            0
        }
        Err(why) => {
            sex_pdx::serial_record!("[gaze.thumb.failed] {}", why);
            1
        }
    };
    Some(status)
}

fn make(words: &[Vec<u8>]) -> Result<(usize, usize), &'static str> {
    let [_, input, output, size] = words else { return Err("usage: --thumb @IN @OUT SIZE") };
    let size = core::str::from_utf8(size).ok().and_then(|s| s.parse::<usize>().ok()).filter(|s| (1..=MAX_SIZE).contains(s))
        .ok_or("SIZE must be a whole number from 1 to 4096")?;
    // The words stay `@/path`: that is how Files sees the grant Loom handed over.
    let (input, output) = (input.as_slice(), output.as_slice());
    if input.is_empty() || output.is_empty() || input == output {
        return Err("the input and the output must be two different files");
    }
    let st = direct::stat(input).map_err(|_| "no such input")?;
    if st.dir || st.size == 0 || st.size > MAX_INPUT {
        return Err("the input is not a photo of a size Gaze reads");
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(st.size as usize).map_err(|_| "out of memory")?;
    bytes.resize(st.size as usize, 0);
    let n = direct::read_at(input, 0, &mut bytes).map_err(|_| "read failed")?;
    bytes.truncate(n);
    let image = gaze_codecs::decode(&bytes, &LIMITS)
        .or_else(|_| sex_image::decode(&bytes, &LIMITS).map_err(gaze_codecs::Error::Image))
        .map_err(|_| "not a picture Gaze can draw")?;
    drop(bytes);
    let image = image.fit_within(size, size).map_err(|_| "out of memory")?;
    let out = encode(&image).ok_or("out of memory")?;
    write_whole(output, &out)?;
    Ok((image.width, image.height))
}

/// The file's bytes: the header, then each pixel (0xAARRGGBB) laid on white as RGB.
fn encode(image: &Image) -> Option<Vec<u8>> {
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize || image.pixels.len() != w * h {
        return None;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(4 + w * h * 3).ok()?;
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    for &p in &image.pixels {
        let a = p >> 24;
        for shift in [16, 8, 0] {
            let c = (p >> shift) & 0xFF;
            out.push(((c * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    Some(out)
}

/// Writes `bytes` to `path` whole, or leaves no file there.
fn write_whole(path: &[u8], bytes: &[u8]) -> Result<(), &'static str> {
    let _ = direct::remove(path);
    let written = direct::create(path).is_ok()
        && matches!(direct::write_at(path, 0, bytes), Ok(n) if n == bytes.len())
        && direct::sync().is_ok()
        && matches!(direct::stat(path), Ok(st) if st.size == bytes.len() as u64);
    if written {
        Ok(())
    } else {
        let _ = direct::remove(path);
        Err("the output could not be written")
    }
}
