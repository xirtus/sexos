#!/bin/sh
# Source-built freestanding ABI qualification; no system plugin/OS loader.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
: "${LD_LLD:=ld.lld}"
RUSTFLAGS='-C relocation-model=pic -C code-model=small' cargo build --manifest-path fixtures/moan-vst-gain/Cargo.toml --release --target "$root/x86_64-sex-app.json" -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem
mkdir -p target/moan-plugin-fixtures
cat > target/moan-plugin-fixtures/vst.exports <<'SYMBOLS'
{global:GetPluginFactory;ModuleEntry;ModuleExit;local:*;};
SYMBOLS
"$LD_LLD" -shared -Bsymbolic -z max-page-size=4096 --version-script=target/moan-plugin-fixtures/vst.exports --whole-archive fixtures/moan-vst-gain/target/x86_64-sex-app/release/libmoan_vst_gain.a --no-whole-archive -o target/moan-plugin-fixtures/gain.vst3
python3 scripts/key_bytes.py target/moan-plugin-fixtures/gain.vst3
