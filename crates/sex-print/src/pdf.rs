// SPDX-License-Identifier: GPL-3.0-only
//! The PDF path: sniffing a PDF to pass it straight to a printer that takes
//! `application/pdf`, and writing plain text as a small PDF (Courier, one font, no
//! compression) for printers that take PDF but not raster.
use crate::caps;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Whether `bytes` is a PDF file (`%PDF-` within the first 1024 bytes, as readers allow).
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

/// Text as a PDF on `media` (a PWG media name), 10 pt Courier with half-inch margins,
/// long lines wrapped. Characters outside printable ASCII become `?`.
pub fn text_to_pdf(text: &str, media: &str) -> Vec<u8> {
    let (w, h) = caps::media_size(media).map(|(w, h)| (caps::hmm_to_points(w), caps::hmm_to_points(h))).unwrap_or((595, 842));
    let (margin, size, leading) = (36u32, 10u32, 12u32);
    let cols = ((w - 2 * margin) * 10 / (size * 6)).max(1) as usize; // Courier advance 0.6 em
    let rows = ((h - 2 * margin) / leading).max(1) as usize;
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let clean: String = raw.trim_end_matches('\r').chars().map(|c| if c == '\t' { ' ' } else if (' '..='~').contains(&c) { c } else { '?' }).collect();
        if clean.is_empty() { lines.push(String::new()); continue; }
        let chars: Vec<char> = clean.chars().collect();
        for chunk in chars.chunks(cols) { lines.push(chunk.iter().collect()); }
    }
    if lines.last().is_some_and(|l| l.is_empty()) && lines.len() > 1 { lines.pop(); }
    let pages: Vec<&[String]> = if lines.is_empty() { alloc::vec![&[][..]] } else { lines.chunks(rows).collect() };

    // Objects: 1 catalog, 2 pages, 3 font, then (page, contents) pairs.
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec());
    for (i, page) in pages.iter().enumerate() {
        let mut s = format!("BT /F1 {size} Tf {leading} TL {margin} {} Td\n", h - margin - size);
        for l in page.iter() {
            let esc: String = l.chars().flat_map(|c| match c { '(' | ')' | '\\' => alloc::vec!['\\', c], c => alloc::vec![c] }).collect();
            s.push_str(&format!("({esc}) Tj T*\n"));
        }
        s.push_str("ET\n");
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + 2 * i).into_bytes());
        let mut c = format!("<< /Length {} >>\nstream\n", s.len()).into_bytes();
        c.extend_from_slice(s.as_bytes());
        c.extend_from_slice(b"endstream");
        objects.push(c);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets { out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes()); }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}
