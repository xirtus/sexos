"""Compile actual Files build pin and staging lease with adversarial candidates."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class HelperImage(unittest.TestCase):
    def test_actual_pin_and_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)
            (tmp / 'src').mkdir()
            (tmp / 'Cargo.toml').write_text('''[package]
name = "restricted-image-test"
version = "0.0.0"
edition = "2021"
[dependencies]
sha2 = { version = "=0.10.9", default-features = false, features = ["force-soft"] }
spin = "0.9.8"
[workspace]
''')
            (tmp / 'moanplug_pin.rs').write_text('pub const LENGTH:usize=0; pub const SHA256:[u8;32]=[0;32];')
            (tmp / 'src/lib.rs').write_text(f'''#[path={str(ROOT / 'servers/sexfiles/build/helper_pin.rs')!r}]
mod pin;
#[path={str(ROOT / 'servers/sexfiles/src/restricted_image.rs')!r}]
mod image;
'''.replace("'", '"') + r'''
#[cfg(test)] mod tests {
use super::*;
use sha2::{Digest,Sha256};
fn identity() -> image::Lease { image::Lease { serial:0,shell_epoch:1,files_epoch:1,parent:42,parent_task:53,cookie:7,nonce:[1;16] } }
#[test] fn build_pin_exact_bytes_and_missing_refused() {
 let path=std::env::temp_dir().join(format!("helper-pin-{}",std::process::id()));
 let _=std::fs::remove_file(&path);
 assert!(pin::pin(&path).is_err());
 let mut elf=vec![0u8;64]; elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01"); elf[18]=62;
 std::fs::write(&path,&elf).unwrap();
 let (n,digest)=pin::pin(&path).unwrap(); assert_eq!(n,64); assert_eq!(digest,<[u8;32]>::from(Sha256::digest(&elf)));
 elf[63]=1; std::fs::write(&path,&elf).unwrap(); assert_ne!(pin::pin(&path).unwrap().1,digest);
 elf[18]=3; std::fs::write(&path,&elf).unwrap(); assert!(pin::pin(&path).is_err());
 std::fs::remove_file(path).unwrap();
}
#[test] fn wrong_helper_race_epoch_replay_fail_closed() {
 let mut state=image::Staging::new(); let data=b"trusted ELF snapshot";
 let hash=Sha256::digest(data).into();
 assert!(state.begin(identity(),b"replaced ELF snapshot",data.len(),hash).is_none());
 assert!(state.begin(identity(),data,data.len()+1,hash).is_none());
 let first=state.begin(identity(),data,data.len(),hash).unwrap(); assert!(state.busy());
 // Another writer/admission cannot replace the kernel staging snapshot.
 assert!(state.begin(identity(),data,data.len(),hash).is_none());
 assert!(!state.copied(first,2,1)); assert!(state.busy());
 assert!(!state.copied(first,1,2)); assert!(state.busy());
 let mut foreign=first; foreign.parent_task+=1; assert!(!state.copied(foreign,1,1));
 assert!(state.copied(first,1,1));
 let second=state.begin(identity(),data,data.len(),hash).unwrap(); assert_ne!(second.serial,first.serial);
 assert!(!state.copied(first,1,1)); assert!(state.busy()); assert!(state.copied(second,1,1));
}
#[test] fn no_guest_pin_means_no_attestation() {
 assert!(image::attest_and_lease(identity(),b"anything").is_none()); assert!(!image::busy());
}
}
''')
            env = os.environ.copy()
            env['OUT_DIR'] = str(tmp)
            env['CARGO_TARGET_DIR'] = str(ROOT / 'target/restricted-image-tests')
            result = subprocess.run(['cargo', 'test', '--offline', '--manifest-path', str(tmp / 'Cargo.toml'), '--lib'], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        source = (ROOT / 'servers/sexfiles/src/program_store.rs').read_text()
        body = source.split('fn try_load(', 1)[1]
        self.assertLess(body.index('restricted_image::busy()'), body.index('memory_region_view('))
        self.assertEqual(source.count('memory_region_view(store::IMAGE_SLOT_FILES'), 1)


if __name__ == '__main__':
    unittest.main()
