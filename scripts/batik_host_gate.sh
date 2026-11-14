#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Batik image editor and digital painting engine host gate.
# Runs unit & golden pixel tests on host and verifies no_std + alloc compilation on x86_64-sex-app.json.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[batik.host.gate] Running host test suite..."
cargo test --locked -p batik-core
cargo test --locked -p batik --no-default-features

echo "[batik.host.gate] Verifying x86_64-sex-app.json target build..."
cargo check --locked -Zbuild-std=core,alloc --target x86_64-sex-app.json -p batik-core
cargo check --locked -Zbuild-std=core,alloc --target x86_64-sex-app.json -p batik

echo "[batik.host.gate] PASS"
