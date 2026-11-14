#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Host gate for sex-codecs: runs locked tests across WebP, GIF, AVIF, HEIC,
# unified streaming animation, and adversarial fuzzing, plus verifies
# #![no_std] compilation on the x86_64-sex-app userspace target.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[sex-codecs-gate] 1/2: Running cargo test --locked -p sex-codecs..."
cargo test --locked -p sex-codecs

echo "[sex-codecs-gate] 2/2: Verifying #![no_std] userspace compilation for x86_64-sex-app.json..."
cargo check --target x86_64-sex-app.json \
  -Z build-std=core,compiler_builtins,alloc \
  -Z build-std-features=compiler-builtins-mem \
  -p sex-codecs

echo "[sex-codecs-gate] PASS: All image decoders, animators and target checks verified."
