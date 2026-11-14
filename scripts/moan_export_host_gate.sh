#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail
cd "$(dirname "$0")/.."
command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null || { echo 'Install host ffmpeg and ffprobe for independent decode checks' >&2; exit 1; }
if [[ ! -f target/moan-host-ffmpeg/sexos-audio-export-v1 ]]; then
  bash scripts/build_moan_host_ffmpeg.sh
fi
cargo test --locked -p moan-export --features host-codecs
