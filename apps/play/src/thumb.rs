// SPDX-License-Identifier: GPL-3.0-only
//! `qupid --thumb @IN @OUT SIZE`: a small picture of a video for Files and Loom, with
//! no window.
//!
//! The same contract as Gaze's and Vellum's: a picture about 10% into the video (or
//! the first one, for a short clip or one that cannot seek), or a song's cover art,
//! fitted inside SIZE by SIZE and never larger than it is, written as
//!
//! ```text
//! width: u16 LE, height: u16 LE, then width * height * 3 bytes of RGB, rows top to bottom
//! ```
//!
//! The output is written whole or not at all, and the run ends with status 0 only when
//! it was written. Sound with no cover art has no picture: status 1 and no file.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::direct;
use sex_media::{Decoder, Next, Source};

/// The largest SIZE accepted: the header holds 16-bit sides.
const MAX_SIZE: usize = 4096;
/// Decoder steps tried for a picture before giving up.
const MAX_STEPS: usize = 4000;

/// The input, read through Files with no window.
struct Input { path: Vec<u8>, len: u64, cache: Vec<u8>, cache_at: u64 }
const CACHE: usize = 256 * 1024;
impl Source for Input {
    fn len(&self) -> Option<u64> { Some(self.len) }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        if at >= self.len { return Some(0); }
        if !(at >= self.cache_at && at < self.cache_at + self.cache.len() as u64) {
            let want = (CACHE as u64).min(self.len - at) as usize;
            self.cache.resize(want, 0);
            match direct::read_at(&self.path, at, &mut self.cache) {
                Ok(got) if got > 0 => { self.cache.truncate(got); self.cache_at = at; }
                _ => return None,
            }
        }
        let off = (at - self.cache_at) as usize;
        let n = out.len().min(self.cache.len() - off);
        out[..n].copy_from_slice(&self.cache[off..off + n]);
        Some(n)
    }
}

/// `Some(status)` when the words start with `--thumb`: the run was a thumbnail and
/// ends with that status. `None` for an ordinary run.
pub fn run(words: &[&[u8]]) -> Option<u32> {
    if words.first().copied() != Some(&b"--thumb"[..]) { return None; }
    let started = sex_pdx::get_ticks();
    let status = match make(words) {
        Ok((w, h, at_ms)) => {
            sex_pdx::serial_record!("[qupid.thumb] w={} h={} at_ms={} ms={}", w, h, at_ms, sex_pdx::get_ticks().saturating_sub(started));
            0
        }
        Err(why) => {
            sex_pdx::serial_record!("[qupid.thumb.failed] {}", why);
            1
        }
    };
    Some(status)
}

fn open(path: &[u8], len: u64) -> Result<Decoder, &'static str> {
    Decoder::open(Box::new(Input { path: path.to_vec(), len, cache: Vec::new(), cache_at: 0 })).map_err(|_| "not a video or song Qupid can open")
}

/// The next picture from `d` into `out`, with its time.
fn picture(d: &mut Decoder, out: &mut [u32], w: usize, h: usize) -> Option<i64> {
    let mut pcm = vec![[0i16; 2]; 2048];
    for _ in 0..MAX_STEPS {
        match d.next(out, w, h, &mut pcm) {
            Ok(Next::Picture { at_ms }) => return Some(at_ms),
            Ok(Next::Sound { .. }) => continue,
            Ok(Next::Later) => { let _ = sex_pdx::sys_wait_self_ticks(1); }
            Ok(Next::End) | Err(_) => return None,
        }
    }
    None
}

fn make(words: &[&[u8]]) -> Result<(usize, usize, i64), &'static str> {
    let [_, input, output, size] = words else { return Err("usage: --thumb @IN @OUT SIZE") };
    let size = core::str::from_utf8(size).ok().and_then(|s| s.parse::<usize>().ok()).filter(|s| (1..=MAX_SIZE).contains(s))
        .ok_or("SIZE must be a whole number from 1 to 4096")?;
    if input.is_empty() || output.is_empty() || input == output {
        return Err("the input and the output must be two different files");
    }
    let st = direct::stat(input).map_err(|_| "no such input")?;
    if st.dir || st.size == 0 { return Err("the input is not a video"); }
    let mut d = open(input, st.size)?;
    let (sw, sh) = (d.info.width as usize, d.info.height as usize);
    if sw == 0 || sh == 0 { return Err("no picture: sound without cover art"); }
    let (w, h) = sex_media::fit(sw, sh, size, size);
    let mut px = Vec::new();
    px.try_reserve_exact(w * h).map_err(|_| "out of memory")?;
    px.resize(w * h, 0u32);
    // About 10% in, past titles and fades; the first picture when that fails (a cover
    // picture, a stream that cannot seek, a clip under a second).
    let duration = d.info.duration_ms;
    let mut at = None;
    if duration >= 1000 && d.seek(duration / 10).is_ok() { at = picture(&mut d, &mut px, w, h); }
    if at.is_none() {
        drop(d);
        let mut again = open(input, st.size)?;
        at = picture(&mut again, &mut px, w, h);
    }
    let at = at.ok_or("no picture in the file")?;
    let mut out = Vec::new();
    out.try_reserve_exact(4 + w * h * 3).map_err(|_| "out of memory")?;
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    for p in &px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
    write_whole(output, &out)?;
    let mid = px[(h / 2) * w + w / 2] & 0xFF_FFFF;
    sex_pdx::serial_record!("[qupid.thumb.written] bytes={} mid={:06x}", out.len(), mid);
    Ok((w, h, at))
}

/// Writes `bytes` to `path` whole, or leaves no file there.
fn write_whole(path: &[u8], bytes: &[u8]) -> Result<(), &'static str> {
    let _ = direct::remove(path);
    let written = direct::create(path).is_ok()
        && matches!(direct::write_at(path, 0, bytes), Ok(n) if n == bytes.len())
        && direct::sync().is_ok()
        && matches!(direct::stat(path), Ok(st) if st.size == bytes.len() as u64);
    if written { Ok(()) } else { let _ = direct::remove(path); Err("the output could not be written") }
}
