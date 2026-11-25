//! Pin the exact Vellum ELF staged into the release image.
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
const MAX_IMAGE: usize = 64 * 1024 * 1024;

pub fn generate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("iso_root/programs/vellum");
    println!("cargo:rerun-if-changed={}", path.display());
    let guest = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none");
    let (len, hash) = if guest {
        let mut bytes = Vec::new();
        fs::File::open(&path).expect("build canonical Vellum before Files")
            .take((MAX_IMAGE + 1) as u64)
            .read_to_end(&mut bytes).expect("read canonical Vellum");
        assert!(bytes.len() >= 64 && bytes.len() <= MAX_IMAGE,
            "canonical Vellum ELF size");
        assert!(&bytes[..7] == b"\x7fELF\x02\x01\x01" && u16::from_le_bytes([bytes[18], bytes[19]]) == 62,
            "canonical Vellum ELF64 x86_64");
        (bytes.len(), Sha256::digest(&bytes).into())
    } else { (0, [0; 32]) };
    let output = Path::new(&std::env::var("OUT_DIR").unwrap()).join("vellum_pin.rs");
    fs::write(output, format!("pub const LENGTH: usize = {len};\npub const SHA256: [u8; 32] = {hash:?};\n")).unwrap();
}
