// SPDX-License-Identifier: GPL-3.0-only
//! welcome.pdf: a two-page document written here, so the viewer has something to show
//! on a fresh disk. It uses the standard fonts every PDF reader carries.

const PAGE_ONE: &str = "\
0.20 0.36 0.70 rg 0 742 612 50 re f
BT /F2 24 Tf 1 1 1 rg 48 758 Td (PDF on SexOS) Tj ET
BT /F1 13 Tf 0.1 0.1 0.12 rg 48 700 Td 18 TL
(This page was drawn by hayro, a PDF renderer written in Rust,) Tj T*
(running as an ordinary Rust program with the standard library.) Tj T*
() Tj T*
(The file came through std::fs from this program's own folder:) Tj T*
(it can read only what its row was granted, nothing else.) Tj T*
() Tj T*
(Keys: Right or Space for the next page, Left for the previous,) Tj T*
(Up and Down to scroll, + and - to zoom, 0 to fit the width.) Tj ET
0.93 0.45 0.20 rg 48 420 160 120 re f
0.25 0.62 0.38 rg 226 420 160 120 re f
0.20 0.36 0.70 rg 404 420 160 120 re f
0.1 0.1 0.12 RG 3 w 48 380 m 564 380 l S
BT /F3 11 Tf 0.3 0.3 0.33 rg 48 350 Td (page 1 of 2) Tj ET
";

const PAGE_TWO: &str = "\
BT /F2 20 Tf 0.1 0.1 0.12 rg 48 730 Td (Second page) Tj ET
BT /F1 13 Tf 0.1 0.1 0.12 rg 48 690 Td 18 TL
(Put any PDF in this program's folder and open it with) Tj T*
(    run pdf NAME.pdf) Tj T*
(Pages, zoom and scrolling work the same for every document.) Tj ET
0.70 0.20 0.36 RG 6 w 306 400 m 406 500 l 506 400 l 406 300 l h S
BT /F3 11 Tf 0.3 0.3 0.33 rg 48 120 Td (page 2 of 2) Tj ET
";

/// The document's bytes.
pub fn document() -> Vec<u8> {
    let fonts = "<< /F1 4 0 R /F2 5 0 R /F3 6 0 R >>";
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font {fonts} >> /Contents 7 0 R >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
        format!("<< /Length {} >>\nstream\n{PAGE_ONE}endstream", PAGE_ONE.len()),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font {fonts} >> /Contents 9 0 R >>"),
        format!("<< /Length {} >>\nstream\n{PAGE_TWO}endstream", PAGE_TWO.len()),
    ];
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets { out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes()); }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn welcome_renders_with_ink_on_both_pages() {
        let pdf = hayro::hayro_syntax::Pdf::new(super::document()).expect("parses");
        assert_eq!(pdf.pages().len(), 2);
        for page in pdf.pages().iter() {
            let pixmap = hayro::render(page, &hayro::RenderCache::new(), &Default::default(), &Default::default(),
                &hayro::PixmapSettings { x_scale: 0.5, y_scale: 0.5, bg_color: hayro::vello_cpu::color::palette::css::WHITE });
            let ink = pixmap.data_as_u8_slice().chunks_exact(4).filter(|p| p[..3] != [255, 255, 255]).count();
            assert!(ink > 500, "page has ink: {ink}");
        }
    }
}
