#!/usr/bin/env bash
# Host gate for Flash (Camera, V7) & SexOS UVC subsystem (G-15)
# Validates sex-uvc crate tests, bare-metal builds for sexuvc & flash,
# and wire-protocol smoke testing with FakeUvcCamera.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
export PATH="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | awk '/^host:/{print $2}')/bin:/opt/homebrew/opt/llvm/bin:$PATH"

echo "[flash.host.gate] 1/4: Running sex-uvc crate host unit and adversarial fuzz tests..."
cargo test --locked -p sex-uvc

echo "[flash.host.gate] 2/4: Building bare-metal sexuvc server..."
for seed in "" 1 2 3 4 5 6 7 8; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p sexuvc --target x86_64-sex.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex/release/sexuvc; then break; fi
  [[ "$seed" != "8" ]] || { echo "[FAIL] sexuvc has refused key bytes"; exit 1; }
done

echo "[flash.host.gate] 3/4: Building release flash app..."
for seed in "" 1 2 3 4 5 6 7 8; do
  SEX_LAYOUT_SEED="$seed" RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --release -p flash --target x86_64-sex-app.json \
    -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
  if python3 scripts/key_bytes.py target/x86_64-sex-app/release/flash; then break; fi
  [[ "$seed" != "8" ]] || { echo "[FAIL] flash has refused key bytes"; exit 1; }
done

mkdir -p iso_root/programs
cp target/x86_64-sex-app/release/flash iso_root/programs/flash

echo "[flash.host.gate] 4/4: Running host smoke test with FakeUvcCamera..."
python3 scripts/flash_host_smoke.py

echo "[flash.host.gate] PASS"
