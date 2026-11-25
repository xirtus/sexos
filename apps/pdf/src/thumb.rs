// SPDX-License-Identifier: GPL-3.0-only
//! `--thumb`: a small picture of a file for Loom and other programs, with no window.
//!
//! `NAME --thumb @/path/in @/path/out SIZE` reads the input, draws it to fit inside
//! SIZE by SIZE keeping its shape (never larger than it is), and writes
//!
//! ```text
//! width: u16 LE, height: u16 LE, then width * height * 3 bytes of RGB, rows top to bottom
//! ```
//!
//! with no padding. Transparent parts are laid on white. The output is written whole or
//! not at all, and the run ends with status 0 only when it was written.
//!
//! Shared by Gaze (apps/view) and Vellum (apps/pdf, through `#[path]`).
use sex_image::Image;

/// The largest SIZE accepted: the header holds 16-bit sides, and a thumbnail bigger
/// than this is not a thumbnail.
pub const MAX_SIZE: usize = 4096;

/// A `--thumb` run's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub input: String,
    pub output: String,
    pub size: usize,
}

/// `Some` when the run was started with `--thumb` (the words after the program's own
/// name); `Err` says what is wrong with the rest.
pub fn request(words: &[String]) -> Option<Result<Request, &'static str>> {
    if words.first().map(String::as_str) != Some("--thumb") {
        return None;
    }
    let [_, input, output, size] = words else { return Some(Err("usage: --thumb @IN @OUT SIZE")) };
    let size = match size.parse::<usize>() {
        Ok(s) if (1..=MAX_SIZE).contains(&s) => s,
        _ => return Some(Err("SIZE must be a whole number from 1 to 4096")),
    };
    if input.is_empty() || output.is_empty() || input == output {
        return Some(Err("the input and the output must be two different files"));
    }
    Some(Ok(Request { input: input.clone(), output: output.clone(), size }))
}

/// The size a `w` by `h` picture is shown at inside `size` by `size`: the same shape,
/// never larger than it is, at least one pixel each way.
pub fn fit(w: usize, h: usize, size: usize) -> (usize, usize) {
    let (w, h, size) = (w.max(1), h.max(1), size.max(1));
    if w <= size && h <= size {
        return (w, h);
    }
    let side = |short: usize, long: usize| ((short * size + long / 2) / long).clamp(1, size);
    if w >= h { (size, side(h, w)) } else { (side(w, h), size) }
}

/// Shrinks `image` to [`fit`] inside `size` by `size`, averaging each source box.
pub fn shrink(image: Image, size: usize) -> Result<Image, sex_image::Error> {
    let (w, h) = fit(image.width, image.height, size);
    resize(image, w, h)
}

/// `image` at exactly `w` by `h`: each output pixel the average of its source box
/// (at least one source pixel, so a side one pixel longer repeats a row or column).
pub fn resize(image: Image, w: usize, h: usize) -> Result<Image, sex_image::Error> {
    if (w, h) == (image.width, image.height) {
        return Ok(image);
    }
    if w == 0 || h == 0 || image.width == 0 || image.height == 0 {
        return Err(sex_image::Error::TooLarge);
    }
    let (sw, sh) = (image.width, image.height);
    let mut out = Vec::new();
    out.try_reserve_exact(w * h).map_err(|_| sex_image::Error::OutOfMemory)?;
    for oy in 0..h {
        let (y0, y1) = (oy * sh / h, ((oy + 1) * sh / h).max(oy * sh / h + 1));
        for ox in 0..w {
            let (x0, x1) = (ox * sw / w, ((ox + 1) * sw / w).max(ox * sw / w + 1));
            // Alpha-weighted, so a transparent pixel's colour does not bleed in.
            let (mut a, mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for &p in &image.pixels[y * sw + x0..y * sw + x1] {
                    let pa = (p >> 24) as u64;
                    a += pa;
                    r += pa * ((p >> 16) & 0xFF) as u64;
                    g += pa * ((p >> 8) & 0xFF) as u64;
                    b += pa * (p & 0xFF) as u64;
                    n += 1;
                }
            }
            out.push(if a == 0 { 0 } else {
                let c = |v: u64| ((v + a / 2) / a) as u32;
                (((a + n / 2) / n) as u32) << 24 | c(r) << 16 | c(g) << 8 | c(b)
            });
        }
    }
    Ok(Image { width: w, height: h, pixels: out })
}

/// The file's bytes: the header, then each pixel (0xAARRGGBB) laid on white as RGB.
pub fn encode(image: &Image) -> Option<Vec<u8>> {
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

/// Stages complete bytes beside `path`, then replaces it through Files' rename.
/// A failed write leaves any previous PDF or thumbnail untouched.
pub fn write_whole(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::{Error, ErrorKind, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut staged = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // An `@/file.pdf` grant may name only that file, not a sibling. Stage in
        // Vellum's own folder and rename to the exact granted destination.
        let name = if path.starts_with("@/") { format!("vellum-{n}.tmp") }
            else { format!("{path}.vellum-{n}.tmp") };
        match OpenOptions::new().write(true).create_new(true).open(&name) {
            Ok(file) => { staged = Some((name, file)); break; }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (name, mut file) = staged.ok_or_else(|| Error::new(ErrorKind::AlreadyExists, "no free Vellum staging name"))?;
    let write_result = (|| {
        file.write_all(bytes)?;
        file.flush()?;
        sync_staged(&file)
    })();
    drop(file);
    let result = write_result.and_then(|()| rename_staged(&name, path));
    if result.is_err() { let _ = std::fs::remove_file(&name); }
    result
}

/// Save a complete PDF and require Files to confirm the promoted name before
/// the viewer may clear its edit history or recovery copy.
pub fn write_confirmed<E: core::fmt::Debug>(
    path: &str, bytes: &[u8], sync: impl FnOnce() -> Result<(), E>,
) -> Result<(), String> {
    write_whole(path, bytes).map_err(|e| format!("Could not write {path}: {e}"))?;
    sync().map_err(|e| format!("Files could not confirm the PDF save: {e:?}"))
}

#[cfg(target_os = "hermit")]
fn sync_staged(file: &std::fs::File) -> std::io::Result<()> {
    use sex_std::fs::FileExt;
    file.fsync()
}

#[cfg(not(target_os = "hermit"))]
fn sync_staged(file: &std::fs::File) -> std::io::Result<()> { file.sync_all() }

#[cfg(target_os = "hermit")]
fn rename_staged(from: &str, to: &str) -> std::io::Result<()> { sex_std::fs::rename(from, to) }

#[cfg(not(target_os = "hermit"))]
fn rename_staged(from: &str, to: &str) -> std::io::Result<()> { std::fs::rename(from, to) }

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> { s.split(' ').map(String::from).collect() }

    #[test]
    fn failed_sync_keeps_filled_form_and_recovery_for_retry() {
        use vellum_edit::{Editor, FieldValue};
        const FORM: &[u8] = include_bytes!("../../../tests/fixtures/pdf/form1.pdf");
        let path = std::env::temp_dir().join(format!("vellum-confirmed-{}.pdf", std::process::id()));
        let path = path.to_str().unwrap();
        let recovery = format!("{path}.vellum-recovered.pdf");
        std::fs::write(path, FORM).unwrap();
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        editor.set_field("name", FieldValue::Text("Ada Lovelace".into())).unwrap();
        let bytes = editor.save_incremental().unwrap();
        std::fs::write(&recovery, &bytes).unwrap();
        let failed = write_confirmed(path, &bytes, || Err("sync failed"));
        assert!(failed.unwrap_err().contains("sync failed"));
        assert!(editor.is_modified(), "unconfirmed form edit must remain dirty");
        assert_eq!(std::fs::read(&recovery).unwrap(), bytes, "recovery copy remains available");
        assert!(write_confirmed(path, &bytes, || Ok::<(), &str>(())).is_ok());
        let reopened = Editor::open(std::fs::read(path).unwrap()).unwrap();
        assert_eq!(reopened.fields().unwrap().into_iter().find(|f| f.name == "name").unwrap().value,
            FieldValue::Text("Ada Lovelace".into()));
        std::fs::remove_file(recovery).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn requests() {
        assert_eq!(request(&words("@/a.png")), None);
        assert_eq!(request(&[]), None);
        assert_eq!(request(&words("--thumb @/a.png @/a.thumb 128")),
            Some(Ok(Request { input: "@/a.png".into(), output: "@/a.thumb".into(), size: 128 })));
        assert!(matches!(request(&words("--thumb @/a.png @/a.thumb")), Some(Err(_))));
        assert!(matches!(request(&words("--thumb @/a.png @/a.thumb 0")), Some(Err(_))));
        assert!(matches!(request(&words("--thumb @/a.png @/a.thumb 5000")), Some(Err(_))));
        assert!(matches!(request(&words("--thumb @/a.png @/a.thumb x")), Some(Err(_))));
        assert!(matches!(request(&words("--thumb @/a.png @/a.png 64")), Some(Err(_))));
        assert!(matches!(request(&words("--thumb @/a.png @/b @/c 64")), Some(Err(_))));
    }

    #[test]
    fn fits_without_growing() {
        assert_eq!(fit(300, 200, 64), (64, 43));
        assert_eq!(fit(200, 300, 64), (43, 64));
        assert_eq!(fit(50, 30, 64), (50, 30));
        assert_eq!(fit(64, 64, 64), (64, 64));
        assert_eq!(fit(1000, 1, 64), (64, 1));
        assert_eq!(fit(1, 1000, 10), (1, 10));
        assert_eq!(fit(612, 792, 128), (99, 128));
    }

    #[test]
    fn header_and_rows() {
        // 3x2: red, half-transparent black, fully transparent; then three greys.
        let image = Image { width: 3, height: 2, pixels: vec![0xFFFF0000, 0x80000000, 0x00123456, 0xFF101010, 0xFF808080, 0xFFFFFFFF] };
        let out = encode(&image).unwrap();
        assert_eq!(&out[..4], &[3, 0, 2, 0]);
        assert_eq!(out.len(), 4 + 3 * 2 * 3);
        assert_eq!(&out[4..7], &[255, 0, 0]);
        assert_eq!(&out[7..10], &[127, 127, 127]);
        assert_eq!(&out[10..13], &[255, 255, 255]);
        assert_eq!(&out[13..22], &[16, 16, 16, 128, 128, 128, 255, 255, 255]);
    }

    #[test]
    fn shrinks_to_the_fitted_size() {
        let image = Image { width: 300, height: 200, pixels: vec![0xFF00FF00; 300 * 200] };
        let small = shrink(image, 64).unwrap();
        assert_eq!((small.width, small.height), (64, 43));
        assert!(small.pixels.iter().all(|p| *p == 0xFF00FF00));
        let tiny = Image { width: 5, height: 4, pixels: vec![0xFF000000; 20] };
        let same = shrink(tiny, 64).unwrap();
        assert_eq!((same.width, same.height), (5, 4));
        // One row short (a renderer rounding down): the last row repeats.
        let short = Image { width: 3, height: 2, pixels: vec![1 << 24, 2 << 24, 3 << 24, 4 << 24, 5 << 24, 6 << 24] };
        let tall = resize(short, 3, 3).unwrap();
        assert_eq!(tall.pixels.iter().map(|p| p >> 24).collect::<Vec<_>>(), [1, 2, 3, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn writes_whole_files() {
        let dir = std::env::temp_dir().join(format!("thumb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.thumb");
        let bytes = encode(&Image { width: 2, height: 1, pixels: vec![0xFF000000, 0xFFFFFFFF] }).unwrap();
        write_whole(path.to_str().unwrap(), &bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // A folder that does not exist: no file, an error.
        let missing = dir.join("no/such/t.thumb");
        assert!(write_whole(missing.to_str().unwrap(), &bytes).is_err());
        assert!(!missing.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replacement_preserves_stale_staging_and_previous_output_until_ready() {
        let dir = std::env::temp_dir().join(format!("vellum-replace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preview.thumb");
        let path = path.to_str().unwrap();
        std::fs::write(path, b"prior thumbnail").unwrap();
        let stale = format!("{path}.vellum-0.tmp");
        std::fs::write(&stale, b"stale bytes").unwrap();
        write_whole(path, b"complete new thumbnail").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"complete new thumbnail");
        assert_eq!(std::fs::read(&stale).unwrap(), b"stale bytes");
        std::fs::remove_file(stale).unwrap();
    }
}
