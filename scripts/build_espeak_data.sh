#!/usr/bin/env bash
# Regenerates crates/sex-speech/espeak-ng-data (eSpeak NG 1.52.0 English data) on the
# host: builds upstream eSpeak NG with CMake, compiles the intonations, phonemes and the
# English dictionary, and copies the six files the voice reads.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
out=crates/sex-speech/espeak-ng-data
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git clone -q --depth 1 --branch 1.52.0 https://github.com/espeak-ng/espeak-ng "$work/src"
cmake -S "$work/src" -B "$work/build" -DUSE_MBROLA=OFF -DUSE_LIBSONIC=OFF -DUSE_LIBPCAUDIO=OFF \
  -DUSE_ASYNC=OFF -DUSE_SPEECHPLAYER=OFF -DESPEAK_BUILD_MANPAGES=OFF >/dev/null
make -C "$work/build" -j"$(nproc)" espeak-ng-bin >/dev/null
b="$work/build"
run() { env ESPEAK_DATA_PATH="$b" "$b/src/espeak-ng" "$@"; }
(cd "$b/phsource" && run --compile-intonations >/dev/null && run --compile-phonemes >/dev/null)
mkdir -p "$b/dictsource"
cp "$work/src/dictsource/en_rules" "$work/src/dictsource/en_list" "$work/src/dictsource/en_emoji" "$b/dictsource/"
(cd "$b/dictsource" && run --compile=en >/dev/null)
mkdir -p "$out/lang/gmw"
for f in phontab phonindex phondata intonations en_dict; do cp "$b/espeak-ng-data/$f" "$out/$f"; done
cp "$b/espeak-ng-data/lang/gmw/en" "$out/lang/gmw/en"
sha256sum "$out"/* "$out/lang/gmw/en" 2>/dev/null | grep -v ': Is a directory'
