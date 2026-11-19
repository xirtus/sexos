// SPDX-License-Identifier: GPL-3.0-only
//! Comic book archives: every picture in the ZIP is a page, in natural name order
//! (`page2` before `page10`). Bodice decodes a page when it is shown, with sex-image.
use crate::zip::Zip;
use crate::{Book, Format, TocEntry};
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

const PICTURES: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp"];

pub fn read(z: &Zip, name: &str) -> Book {
    let mut images: Vec<String> = z.entries.iter().map(|e| e.name.clone())
        .filter(|n| !n.ends_with('/') && !n.starts_with("__MACOSX/") && !n.rsplit('/').next().unwrap_or("").starts_with('.'))
        .filter(|n| { let l = n.to_ascii_lowercase(); PICTURES.iter().any(|p| l.ends_with(p)) })
        .collect();
    images.sort_by(|a, b| natural(a, b));
    let title = String::from(name.rsplit('/').next().unwrap_or(name).trim_end_matches(".cbz").trim_end_matches(".CBZ"));
    // A folder per chapter, when the archive has folders.
    let mut toc = Vec::new();
    let mut last = "";
    for (i, n) in images.iter().enumerate() {
        let dir = crate::folder(n);
        if dir != last && !dir.is_empty() { toc.push(TocEntry { label: String::from(dir.trim_end_matches('/')), chapter: i, depth: 0 }); }
        last = dir;
    }
    Book { format: Format::Cbz, title, author: String::new(), chapters: Vec::new(), toc, images }
}

/// Names compared with runs of digits as numbers, letters without case.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let nx = x.iter().take_while(|c| c.is_ascii_digit()).count();
                let ny = y.iter().take_while(|c| c.is_ascii_digit()).count();
                let (dx, dy) = (trim_zeros(&x[..nx]), trim_zeros(&y[..ny]));
                let o = dx.len().cmp(&dy.len()).then(dx.cmp(dy));
                if o != Ordering::Equal { return o; }
                x = &x[nx..];
                y = &y[ny..];
            }
            (Some(p), Some(q)) => {
                let o = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
                if o != Ordering::Equal { return o; }
                x = &x[1..];
                y = &y[1..];
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let n = d.iter().take_while(|c| **c == b'0').count();
    &d[n.min(d.len().saturating_sub(1))..]
}
