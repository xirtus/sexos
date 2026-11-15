#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The Rust standard library source ordinary SexOS programs (x86_64-sex-std.json) build.

std's Hermit port, which that target uses, has no process support: `std::process` is
std's `unsupported` module, which never calls the program's library, so no answer in
crates/sex-std could make `Command` work. This script copies the toolchain's `library/`
(rust-src) to target/sex-std-src/library and adds one file,
patches/rust-std/process_sexos.rs, as std/src/sys/process/sexos.rs, chosen for
`target_os = "hermit"`. Nothing else in std changes. The build trace points cargo's
`-Z build-std` at the copy (`__CARGO_TESTS_ONLY_SRC_ROOT`) and std then builds as usual.

Prints the copy's `library` path. Does nothing when the copy is already current."""
import hashlib,os,shutil,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OVERLAY=ROOT/'patches/rust-std/process_sexos.rs'
ANCHOR="""    _ => {
        mod unsupported;
        use unsupported as imp;
    }"""
ARM="""    target_os = "hermit" => {
        // SexOS (patches/rust-std/process_sexos.rs, scripts/sex_std_source.py).
        mod sexos;
        use sexos as imp;
    }
"""
def main():
 out=Path(sys.argv[1]) if len(sys.argv)>1 else ROOT/'target/sex-std-src'
 sysroot=subprocess.run(['rustc','--print','sysroot'],cwd=ROOT,check=True,capture_output=True,text=True).stdout.strip()
 source=Path(sysroot)/'lib/rustlib/src/rust/library'
 mod=source/'std/src/sys/process/mod.rs'
 if not mod.exists():raise SystemExit(f'rust-src is missing: {mod} (rustup component add rust-src)')
 overlay=OVERLAY.read_bytes();original=mod.read_text()
 stamp=hashlib.sha256(str(source).encode()+original.encode()+overlay+Path(__file__).read_bytes()).hexdigest()
 library=out/'library';mark=out/'stamp'
 if mark.exists() and mark.read_text()==stamp and (library/'std/src/sys/process/sexos.rs').exists():
  print(library);return
 if original.count(ANCHOR)!=1:raise SystemExit(f'{mod}: std changed; update scripts/sex_std_source.py for this toolchain')
 shutil.rmtree(out,ignore_errors=True);out.mkdir(parents=True)
 try:subprocess.run(['cp','-al',str(source),str(library)],check=True,stderr=subprocess.DEVNULL)
 except subprocess.CalledProcessError:
  shutil.rmtree(library,ignore_errors=True);shutil.copytree(source,library,symlinks=True)
 # Hard links share the toolchain's bytes: the changed file is replaced, never written through.
 target=library/'std/src/sys/process/mod.rs';target.unlink()
 target.write_text(original.replace(ANCHOR,ARM+ANCHOR))
 (library/'std/src/sys/process/sexos.rs').write_bytes(overlay)
 mark.write_text(stamp)
 print(library)
if __name__=='__main__':main()
