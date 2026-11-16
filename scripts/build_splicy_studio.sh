#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds Splicy Studio: Splicy's editor (core/: engine and egui interface) on sex-egui, for
# SexOS's standard library (x86_64-sex-std.json). Its own workspace (apps/splicy-studio),
# size-optimised with fat LTO to fit the 32 MiB program image.
# Output: apps/splicy-studio/target/x86_64-sex-std/release/splicy-studio.
set -euo pipefail
cd "$(dirname "$0")/.."
# Splicy lives in its own repository (github.com/xirtus/Splicy), checked out here as a submodule.
[[ -f apps/splicy-studio/Cargo.toml ]] || git submodule update --init apps/splicy-studio
src="$(python3 scripts/sex_std_source.py)"
bin=apps/splicy-studio/target/x86_64-sex-std/release/splicy-studio
build() {
  (cd apps/splicy-studio && SEX_LAYOUT_SEED="$1" __CARGO_TESTS_ONLY_SRC_ROOT="$src" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release \
    -Z build-std=std,panic_abort -Z build-std-features=compiler-builtins-mem \
    --target ../../x86_64-sex-std.json)
}
# The loader refuses code holding WRPKRU/XRSTOR bytes (scripts/key_bytes.py): link again
# with the sections in another order until none are left.
seed=0
build $seed
while ! python3 scripts/key_bytes.py "$bin"; do
  seed=$((seed + 1)); [[ $seed -le 24 ]] || { echo "no clean layout found" >&2; exit 1; }
  build $seed
done
size=$(stat -c %s "$bin")
[[ $size -le 33554432 ]] || { echo "splicy-studio is $size bytes, over the 32 MiB program image" >&2; exit 1; }
ls -la "$bin"
