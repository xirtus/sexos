#!/usr/bin/env bash
# Sex AArch64 Phase-2 build — entry + PL011 serial ONLY.
# Separate Arm-only path: no Limine, no ISO flow, no x86 target changes.
# Mirrors the canonical invocation shape of build_payload.sh.
# Expected artifact: target/aarch64-sex/release/sex-kernel-arm64
# Expected ELF: machine EM_AARCH64 (183), entry 0x40080000.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo >/dev/null || { echo "[build_arm64] FATAL: cargo missing"; exit 1; }
[ -f aarch64-sex.json ] || { echo "[build_arm64] FATAL: aarch64-sex.json missing"; exit 1; }
[ -f kernel/aarch64-linker.ld ] || { echo "[build_arm64] FATAL: kernel/aarch64-linker.ld missing"; exit 1; }
[ -f kernel/src/arch/aarch64/main.rs ] || { echo "[build_arm64] FATAL: aarch64 entry missing"; exit 1; }
rustc --version | grep -q nightly || echo "[build_arm64] WARN: non-nightly rustc; build-std may fail"

RUSTFLAGS="-C link-arg=-Tkernel/aarch64-linker.ld" cargo build \
  -Z build-std=core,compiler_builtins,alloc \
  -Z build-std-features=compiler-builtins-mem \
  --package sex-kernel \
  --bin sex-kernel-arm64 \
  --features sex-kernel/arm64 \
  --target aarch64-sex.json \
  --release

IMG=target/aarch64-sex/release/sex-kernel-arm64
[ -f "$IMG" ] || { echo "[build_arm64] FATAL: image not produced"; exit 1; }
echo "[build_arm64] image: $IMG"
file "$IMG" 2>/dev/null || true
