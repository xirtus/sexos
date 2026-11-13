// SPDX-License-Identifier: GPL-3.0-only
//! Web fonts: a page's `@font-face` rules name font files, which load with the page
//! (TrueType, OpenType or WOFF; WOFF2 is skipped for another source the rule lists).
//! Each file is kept once for the program's life, by address, and its family becomes
//! one that `font-family` can name.

use super::Browser;
use alloc::string::String;
use alloc::vec::Vec;
use super::sex_html;
use sex_html::css::FontFace;

/// Most font files one page loads.
const PER_PAGE: usize = 12;

/// The source to fetch: the first in a format that can be read.
fn usable(f: &FontFace) -> Option<&str> {
    f.sources.iter().find(|(url, format)| {
        let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        match format.as_str() {
            "truetype" | "opentype" | "woff" | "truetype-variations" | "opentype-variations" | "woff-variations" => true,
            "" => !url.starts_with("data:") && (path.ends_with(".ttf") || path.ends_with(".otf") || path.ends_with(".woff")),
            _ => false,
        }
    }).map(|(u, _)| u.as_str())
}

impl Browser {
    /// Loads the fonts `faces` name ((the address their sheet came from, rule)) and adds
    /// their families.
    pub(super) fn load_fonts(&mut self, faces: Vec<(String, FontFace)>) {
        let mut found = Vec::new();
        let mut fetched = 0;
        for (base, face) in faces {
            let Some(src) = usable(&face) else { continue };
            let Some(url) = sex_html::url::resolve(&base, src) else { continue };
            let number = match self.web_fonts.iter().find(|(u, _)| *u == url) {
                Some((_, n)) => *n,
                None => {
                    if fetched >= PER_PAGE { continue; }
                    fetched += 1;
                    self.status = Some("Loading fonts…");
                    let n = match self.fetch(&url) {
                        Ok((_, body)) => sex_fetch::woff::to_sfnt(&self.buf[body]).and_then(|ttf| self.fonts.add_face(ttf)),
                        Err(_) => None,
                    };
                    self.buf = Vec::new();
                    sex_pdx::serial_record!("[web.font] family={} loaded={} url={}", face.family, n.is_some() as u8, url);
                    self.web_fonts.push((url, n));
                    n
                }
            };
            if let Some(n) = number { found.push((face.family, face.bold, face.italic, n)); }
        }
        if !found.is_empty() { sex_html::style::add_web_fonts(found); }
    }
}
