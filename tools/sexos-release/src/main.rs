// SPDX-License-Identifier: GPL-3.0-only
//! sexos-release: the release key and signed update indexes for SexOS `update`.
//!
//!   sexos-release keygen DIR
//!       New key pair: DIR/release.key (secret, keep it off the repository) and
//!       DIR/release.pub (commit it as keys/release.pub; builds then trust it).
//!   sexos-release sign KEY VERSION IMAGE OUT_DIR
//!       Writes OUT_DIR/sexos-update.txt (version, size, SHA-256 of IMAGE) and
//!       OUT_DIR/sexos-update.sig. Upload both with IMAGE named sexos-boot.img.
//!       KEY is a release.key file, "-" for the SEXOS_RELEASE_SECRET variable, or "test"
//!       for the gates' public test key (trusted only by builds told to trust it).
//!   sexos-release public KEY
//!       Prints KEY's public half: what SEXOS_RELEASE_KEY or keys/release.pub holds.
//!   sexos-release verify PUB DIR
//!       Checks DIR's index and signature against PUB and the image beside them.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::exit;

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn die(msg: &str) -> ! { eprintln!("sexos-release: {msg}"); exit(1) }
fn read_hex<const N: usize>(text: &str, what: &str) -> [u8; N] {
    let line: String = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    sex_disk::update::unhex::<N>(line.trim().as_bytes()).unwrap_or_else(|| die(&format!("{what} is not {} hex digits", 2 * N)))
}
/// The gates' key. Its secret is public on purpose: only a build made with
/// SEXOS_RELEASE_KEY set to its public half trusts it.
const TEST_SEED: [u8; 32] = *b"SexOS update gates: test only!!!";
fn secret(key: &str) -> [u8; 32] {
    match key {
        "test" => TEST_SEED,
        "-" => read_hex::<32>(&std::env::var("SEXOS_RELEASE_SECRET").unwrap_or_else(|_| die("SEXOS_RELEASE_SECRET is not set")), "SEXOS_RELEASE_SECRET"),
        path => read_hex::<32>(&String::from_utf8(read(Path::new(path))).unwrap_or_else(|_| die("key file is not text")), "the key"),
    }
}
fn read(path: &Path) -> Vec<u8> { std::fs::read(path).unwrap_or_else(|e| die(&format!("{}: {e}", path.display()))) }
fn write(path: &Path, data: &[u8]) { std::fs::write(path, data).unwrap_or_else(|e| die(&format!("{}: {e}", path.display()))) }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", dir] => {
            let dir = PathBuf::from(dir);
            let (key, public) = (dir.join("release.key"), dir.join("release.pub"));
            if key.exists() { die(&format!("{} exists; not replacing a release key", key.display())); }
            let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::generate());
            write(&key, format!("# SexOS release secret key. Never commit or share it.\n{}\n", hex(kp.sk.seed().as_ref())).as_bytes());
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)); }
            write(&public, format!("# SexOS release public key (Ed25519)\n{}\n", hex(kp.pk.as_ref())).as_bytes());
            println!("{}\n{}", key.display(), public.display());
        }
        ["sign", key, version, image, out] => {
            let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(secret(key)));
            let version: u64 = version.parse().unwrap_or_else(|_| die("VERSION must be a number"));
            let img = read(Path::new(image));
            if img.is_empty() || img.len() % 512 != 0 { die("the image must be whole 512-byte sectors (make_boot_image.py output)"); }
            let index = format!("# SexOS release\nversion={version}\nimage=sexos-boot.img\nsha256={}\nbytes={}\n", hex(&Sha256::digest(&img)), img.len());
            let sig = kp.sk.sign(index.as_bytes(), None);
            let out = PathBuf::from(out);
            write(&out.join("sexos-update.txt"), index.as_bytes());
            write(&out.join("sexos-update.sig"), format!("{}\n", hex(sig.as_ref())).as_bytes());
            print!("{index}");
        }
        ["public", key] => {
            println!("{}", hex(ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(secret(key))).pk.as_ref()));
        }
        ["verify", public, dir] => {
            let pk = read_hex::<32>(&String::from_utf8(read(Path::new(public))).unwrap_or_default(), "the public key");
            let dir = PathBuf::from(dir);
            let (index, sig) = (read(&dir.join("sexos-update.txt")), read(&dir.join("sexos-update.sig")));
            let idx = sex_disk::update::verify_index(&index, &sig, &pk).unwrap_or_else(|e| die(&format!("index: {e:?}")));
            let img = read(&dir.join("sexos-boot.img"));
            if img.len() as u64 != idx.bytes || <[u8; 32]>::from(Sha256::digest(&img)) != idx.sha256 { die("sexos-boot.img is not the image the index names"); }
            println!("good: version {} ({} bytes)", idx.version, idx.bytes);
        }
        _ => { eprintln!("usage: sexos-release keygen DIR | sign KEY VERSION IMAGE OUT_DIR | public KEY | verify PUB DIR"); exit(2) }
    }
}
