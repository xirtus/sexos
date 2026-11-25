//! Independent, byte-exact verification of a Vellum signing request.
use sha2::{Digest, Sha256};
use vellum_edit::{cms, Editor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure { Path, Document }

/// Refuse invisible formatting that could reverse or disguise the name shown
/// in the signer's own approval view.
pub(crate) fn safe_label(text: &str) -> bool {
    !text.chars().any(|c| c.is_control() || (c.is_whitespace() && c != ' ') || matches!(c,
        '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180b}'..='\u{180f}' |
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' |
        '\u{fe00}'..='\u{fe0f}' | '\u{feff}'))
}

pub(crate) fn canonical_path(path: &str) -> bool {
    path.starts_with("@/") && path.len() > 2 && safe_label(path)
        && !path.bytes().any(|b| b == b'\\')
        && path[2..].split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn signed_attributes(bytes: &[u8], field: &str, cert: &[u8], point: &[u8]) -> Result<Vec<u8>, Failure> {
    let editor = Editor::open(bytes.to_vec()).map_err(|_| Failure::Document)?;
    if editor.is_modified() { return Err(Failure::Document); }
    let mut attrs = None;
    let _ = editor.embed_detached_signature_with(field, 4096, |first, second| {
        let _ = cms::detached_p256_sha256(first, second, cert, point, |candidate| {
            attrs = Some(candidate.to_vec());
            Err(cms::CmsError::SignerRefused)
        });
        Err(vellum_edit::Error::BadValue("preview only".into()))
    });
    attrs.ok_or(Failure::Document)
}

/// The selected path is an independent Files grant to Shibari. Vellum's path,
/// field and attributes are untrusted until all three match this read.
pub(crate) fn verify(
    claimed_path: &str, selected_path: &str, bytes: &[u8], field: &str,
    cert: &[u8], point: &[u8], requested_attrs: &[u8],
) -> Result<[u8; 32], Failure> {
    if !canonical_path(claimed_path) || claimed_path != selected_path { return Err(Failure::Path); }
    if signed_attributes(bytes, field, cert, point)? != requested_attrs { return Err(Failure::Document); }
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, obj).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for off in offs { out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes()); }
        out.extend_from_slice(format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        let mut editor = Editor::open(out).unwrap();
        editor.add_signature_field(0, "Approval", [20.0, 20.0, 120.0, 50.0]).unwrap();
        editor.save_incremental().unwrap()
    }

    #[test]
    fn rejects_forged_attrs_swapped_file_or_field() {
        let bytes = pdf();
        let cert = include_bytes!("../../../crates/vellum-edit/tests/fixtures/cms-test-cert.der");
        let point = cms::certificate_public_point(cert).unwrap();
        let attrs = signed_attributes(&bytes, "Approval", cert, point).unwrap();
        assert!(verify("@/home/a.pdf", "@/home/a.pdf", &bytes, "Approval", cert, point, &attrs).is_ok());
        let mut forged = attrs.clone(); forged[3] ^= 1;
        assert_eq!(verify("@/home/a.pdf", "@/home/a.pdf", &bytes, "Approval", cert, point, &forged), Err(Failure::Document));
        assert_eq!(verify("@/home/a.pdf", "@/home/b.pdf", &bytes, "Approval", cert, point, &attrs), Err(Failure::Path));
        assert_eq!(verify("@/home/a.pdf", "@/home/a.pdf", &bytes, "Other", cert, point, &attrs), Err(Failure::Document));
        assert!(!canonical_path("@/home/../a.pdf"));
        assert!(!canonical_path("/home/a.pdf"));
        assert!(!canonical_path("@/home/a\u{202e}fdp"));
        assert!(!canonical_path("@/home/a\u{200b}.pdf"));
        assert!(!safe_label("Approval\u{2028}Press Y"));
        assert!(!safe_label("Approval\u{206a}Other"));
    }

    #[test]
    fn source_change_before_or_after_approval_invalidates_request() {
        let bytes = pdf();
        let cert = include_bytes!("../../../crates/vellum-edit/tests/fixtures/cms-test-cert.der");
        let point = cms::certificate_public_point(cert).unwrap();
        let attrs = signed_attributes(&bytes, "Approval", cert, point).unwrap();
        let accepted = verify("@/home/a.pdf", "@/home/a.pdf", &bytes, "Approval", cert, point, &attrs).unwrap();
        let mut changed = bytes.clone(); changed.extend_from_slice(b"\n% another revision\n");
        assert_eq!(verify("@/home/a.pdf", "@/home/a.pdf", &changed, "Approval", cert, point, &attrs), Err(Failure::Document));
        let reread = verify("@/home/a.pdf", "@/home/a.pdf", &bytes, "Approval", cert, point, &attrs).unwrap();
        assert_eq!(accepted, reread);
    }
}
