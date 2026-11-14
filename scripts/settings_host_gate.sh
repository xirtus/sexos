#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Settings on the host: the sex-config schema, file format and protocol, and the
# Settings program's pages, search and value steps (apps/settings/src/model.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -q --locked -p sex-config -p sex-time
cargo test -q --locked -p sex-config --features password
out="$(mktemp -d)"; trap 'rm -rf "$out"' EXIT
rustc --edition 2021 --crate-type rlib --crate-name sex_config crates/sex-config/src/lib.rs -o "$out/libsex_config.rlib"
rustc --edition 2021 --test -A dead_code --extern sex_config="$out/libsex_config.rlib" apps/settings/src/model.rs -o "$out/settings-model"
"$out/settings-model" -q
rustc --edition 2021 --test -A dead_code --extern sex_config="$out/libsex_config.rlib" apps/settings/src/setup.rs -o "$out/settings-setup"
"$out/settings-setup" -q
rustc --edition 2021 --test -A dead_code tests/screen_state.rs -o "$out/screen-state" && "$out/screen-state" -q
echo "[settings.host.gate] PASS"
