//! Hand-built fixture PDFs.

#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    /// Classic xref table.
    Table,
    /// Xref stream (Flate + PNG Up predictor), with non-stream objects packed in an object stream.
    XrefStream,
}

/// Objects are (number, body). Bodies of stream objects must contain `stream`.
pub fn build(objects: &[(u32, String)], style: Style, root: u32) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let size = objects.iter().map(|o| o.0).max().unwrap() + 1;
    match style {
        Style::Table => {
            let mut offs = vec![0usize; size as usize];
            for (n, body) in objects {
                offs[*n as usize] = out.len();
                out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
            }
            let x = out.len();
            out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
            for o in &offs[1..] {
                out.extend_from_slice(if *o == 0 { "0000000000 65535 f \n".to_string() } else { format!("{o:010} 00000 n \n") }.as_bytes());
            }
            out.extend_from_slice(format!("trailer\n<< /Size {size} /Root {root} 0 R /ID [<0011223344556677> <0011223344556677>] >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
        }
        Style::XrefStream => {
            let stm = size;
            let xref = size + 1;
            // entries: (type, f2, f3)
            let mut entries = vec![(0u8, 0u64, 65535u64); (xref + 1) as usize];
            let mut header = String::new();
            let mut body = String::new();
            let mut idx = 0;
            for (n, b) in objects {
                if b.contains("stream") {
                    entries[*n as usize] = (1, out.len() as u64, 0);
                    out.extend_from_slice(format!("{n} 0 obj\n{b}\nendobj\n").as_bytes());
                } else {
                    header.push_str(&format!("{n} {} ", body.len()));
                    body.push_str(b);
                    body.push('\n');
                    entries[*n as usize] = (2, stm as u64, idx);
                    idx += 1;
                }
            }
            let data = format!("{header}{body}");
            let z = miniz_oxide::deflate::compress_to_vec_zlib(data.as_bytes(), 6);
            entries[stm as usize] = (1, out.len() as u64, 0);
            out.extend_from_slice(format!("{stm} 0 obj\n<< /Type /ObjStm /N {idx} /First {} /Filter /FlateDecode /Length {} >>\nstream\n", header.len(), z.len()).as_bytes());
            out.extend_from_slice(&z);
            out.extend_from_slice(b"\nendstream\nendobj\n");
            let x = out.len();
            entries[xref as usize] = (1, x as u64, 0);
            // Rows of 1+4+2 bytes, PNG Up predictor.
            let mut raw = Vec::new();
            let mut prev = [0u8; 7];
            for (t, a, b) in &entries {
                let mut row = [0u8; 7];
                row[0] = *t;
                row[1..5].copy_from_slice(&(*a as u32).to_be_bytes());
                row[5..7].copy_from_slice(&(*b as u16).to_be_bytes());
                raw.push(2);
                for i in 0..7 { raw.push(row[i].wrapping_sub(prev[i])); }
                prev = row;
            }
            let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
            out.extend_from_slice(format!("{xref} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root {root} 0 R /ID [<0011223344556677> <0011223344556677>] /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >> /Length {} >>\nstream\n", xref + 1, z.len()).as_bytes());
            out.extend_from_slice(&z);
            out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{x}\n%%EOF\n").as_bytes());
        }
    }
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// One 200x200 page with a grey bar and some text.
pub fn simple(style: Style) -> Vec<u8> {
    let content = "0.5 g 20 20 160 10 re f\nBT /F1 18 Tf 0 g 20 150 Td (Hello Vellum) Tj ET";
    let objs = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string()),
        (3, "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string()),
        (4, stream("", content)),
        (5, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string()),
    ];
    build(&objs, style, 1)
}

/// A one-page form: person.name (text), person.age (read-only text), agree (checkbox with
/// appearances), color (radio Red/Blue), size (combo S/M/L), notes (multiline text without
/// any widget appearance) and, when `signed`, a signature field `sig` with a signature dictionary.
pub fn form(style: Style, signed: bool) -> Vec<u8> {
    let on = stream("/Type /XObject /Subtype /Form /BBox [0 0 20 20]", "0 g 4 4 12 12 re f");
    let off = stream("/Type /XObject /Subtype /Form /BBox [0 0 20 20]", "0 G 1 w 0.5 0.5 19 19 re S");
    let mut objs = vec![
        (1, format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 9 0 R 12 0 R 15 0 R 17 0 R{}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>", if signed { " 18 0 R" } else { "" })),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string()),
        (3, format!("<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [7 0 R 8 0 R 9 0 R 13 0 R 14 0 R 15 0 R 17 0 R{}] >>", if signed { " 18 0 R" } else { "" })),
        (4, stream("", "BT /F1 10 Tf 0 g 10 185 Td (Form) Tj ET")),
        (5, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string()),
        (6, "<< /T (person) /Kids [7 0 R 8 0 R] >>".to_string()),
        (7, "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /T (name) /FT /Tx /DA (/Helv 12 Tf 0 0 1 rg) /Rect [20 150 180 170] /F 4 /P 3 0 R /MK << /BC [0 0 0] >> >>".to_string()),
        (8, "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /T (age) /FT /Tx /Ff 1 /V (42) /DA (/Helv 10 Tf 0 g) /Rect [20 125 60 145] /F 4 /P 3 0 R >>".to_string()),
        (9, "<< /Type /Annot /Subtype /Widget /T (agree) /FT /Btn /V /Off /AS /Off /Rect [20 95 40 115] /F 4 /P 3 0 R /AP << /N << /Yes 10 0 R /Off 11 0 R >> >> >>".to_string()),
        (10, on.clone()),
        (11, off.clone()),
        (12, "<< /T (color) /FT /Btn /Ff 49152 /V /Red /Kids [13 0 R 14 0 R] >>".to_string()),
        (13, "<< /Type /Annot /Subtype /Widget /Parent 12 0 R /AS /Red /Rect [60 95 80 115] /F 4 /P 3 0 R /AP << /N << /Red 10 0 R /Off 11 0 R >> >> >>".to_string()),
        (14, "<< /Type /Annot /Subtype /Widget /Parent 12 0 R /AS /Off /Rect [90 95 110 115] /F 4 /P 3 0 R /AP << /N << /Blue 10 0 R /Off 11 0 R >> >> >>".to_string()),
        (15, "<< /Type /Annot /Subtype /Widget /T (size) /FT /Ch /Ff 131072 /Opt [(S) [(M) (Medium)] (L)] /V (S) /DA (/Helv 0 Tf 0 g) /Rect [120 95 180 115] /F 4 /P 3 0 R >>".to_string()),
        (16, "null".to_string()),
        (17, "<< /Type /Annot /Subtype /Widget /T (notes) /FT /Tx /Ff 4096 /DA (/Helv 9 Tf 0 g) /Rect [20 20 180 80] /F 4 /P 3 0 R >>".to_string()),
    ];
    if signed {
        objs.push((18, "<< /Type /Annot /Subtype /Widget /T (sig) /FT /Sig /V 19 0 R /Rect [0 0 0 0] /F 4 /P 3 0 R >>".to_string()));
        objs.push((19, "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /Name (Ada Lovelace) /M (D:20261114093000+01'00') /Reason (Approved) /ByteRange [0 0000000000 0000000000 0000000000] /Contents <00000000000000000000000000000000> >>".to_string()));
    }
    let mut pdf = build(&objs, style, 1);
    if signed && style == Style::Table {
        // Make the byte range real: everything except the /Contents hex string, to end of file.
        let find = |h: &[u8], n: &[u8]| h.windows(n.len()).position(|w| w == n).unwrap();
        let c = find(&pdf, b"/Contents <0000") + 10;
        let c_end = c + 34;
        let br = find(&pdf, b"/ByteRange [0 ") + 14;
        let text = format!("{:010} {:010} {:010}", c, c_end, pdf.len() - c_end);
        pdf[br..br + 32].copy_from_slice(text.as_bytes());
    }
    pdf
}
