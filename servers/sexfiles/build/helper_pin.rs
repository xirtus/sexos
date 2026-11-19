//! Derive trust from the canonical build artifact, never a caller-provided digest.
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub const MAX_HELPER: usize = 16 * 1024 * 1024;

pub fn pin(path: &Path) -> Result<(usize, [u8; 32]), String> {
    // Read at most the admitted ELF size plus a probe byte, including growth races.
    use std::io::Read;
    let file =
        fs::File::open(path).map_err(|e| format!("trusted moanplug artifact missing: {e}"))?;
    let mut bytes = Vec::new();
    file.take((MAX_HELPER + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() < 64
        || bytes.len() > MAX_HELPER
        || &bytes[..7] != b"\x7fELF\x02\x01\x01"
        || u16::from_le_bytes([bytes[18], bytes[19]]) != 62
    {
        return Err("trusted moanplug must be bounded ELF64 little-endian x86_64".into());
    }
    Ok((bytes.len(), Sha256::digest(&bytes).into()))
}

pub fn generate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = root.join("iso_root/programs/moanplug");
    println!("cargo:rerun-if-changed={}", helper.display());
    // Files is a freestanding server. Native model builds do not publish a server.
    let guest = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none");
    let (len, hash) = if guest {
        pin(&helper)
            .unwrap_or_else(|e| panic!("{e}; build and scan canonical moanplug before Files"))
    } else {
        (0, [0; 32])
    };
    let output = Path::new(&std::env::var("OUT_DIR").unwrap()).join("moanplug_pin.rs");
    fs::write(
        output,
        format!("pub const LENGTH: usize = {len};\npub const SHA256: [u8; 32] = {hash:?};\n"),
    )
    .unwrap();
}
