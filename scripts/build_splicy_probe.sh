#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds splicy-probe: FilmCraft's editing engine run headless on SexOS's standard
# library (x86_64-sex-std.json). Output: target/x86_64-sex-std/release/splicy-probe.
set -euo pipefail
cd "$(dirname "$0")/.."
src="$(python3 scripts/sex_std_source.py)"
__CARGO_TESTS_ONLY_SRC_ROOT="$src" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release \
  -Z build-std=std,panic_abort -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-std.json -p splicy-probe
bin=target/x86_64-sex-std/release/splicy-probe
python3 scripts/key_bytes.py "$bin"
ls -la "$bin"
