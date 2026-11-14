#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# core_models_host_gate.sh — Host regressions for core user-space data models,
# crypto, volume parsers, and software graphics engines.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[CORE MODELS HOST GATE] Testing silkbar-model, sex-object-model, sex-trust-config, sex-volumes, sex-crypt, sex-canvas, sex-gl..."

cargo test --locked \
    -p silkbar-model \
    -p sex-object-model \
    -p sex-trust-config \
    -p sex-volumes \
    -p sex-crypt \
    -p sex-canvas \
    -p sex-gl

echo "[CORE MODELS HOST GATE] PASS"
