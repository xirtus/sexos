#!/usr/bin/env bash
# Bell's rate conversion on the host: pitch, length and seamless joins.
set -euo pipefail
cd "$(dirname "$0")/.."
TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test tests/bell_resample.rs -o "$TEST_DIR/bell-resample"
"$TEST_DIR/bell-resample"
rustc --edition=2021 --test tests/bell_jack.rs -o "$TEST_DIR/bell-jack"
"$TEST_DIR/bell-jack"
rustc --edition=2021 --test tests/bell_hdmi.rs -o "$TEST_DIR/bell-hdmi"
"$TEST_DIR/bell-hdmi"
rustc --edition=2021 --test tests/usb_audio.rs -o "$TEST_DIR/usb-audio"
"$TEST_DIR/usb-audio"
