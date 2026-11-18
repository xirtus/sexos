#!/usr/bin/env bash
# ci_image_pack.sh OUT.tar.zst — pack one CI build for the jobs that only boot it.
#
# CI builds the image once (job `image`); every gate job that tests the default
# image unpacks this instead of spending ~25 minutes rebuilding the same thing.
# It holds the ISO, iso_root/ and each target/*/release's own outputs (programs,
# kernel, host tools), not cargo's intermediate files (deps, build scripts,
# fingerprints), which the jobs' cargo cache restores when they build anything.
set -euo pipefail
out="${1:?usage: ci_image_pack.sh OUT.tar.zst}"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test -f sexos-v1.0.0.iso || { echo "[ci.image] no sexos-v1.0.0.iso: build first" >&2; exit 1; }
mapfile -t outputs < <(find target -mindepth 2 -maxdepth 3 -type f \
  \( -path 'target/*/release/*' -o -path 'target/release/*' -o -path 'target/*/debug/*' \) \
  ! -name '*.d' ! -name '*.rlib' ! -name '*.rmeta' ! -name '.cargo-lock' | sort)
tar -I 'zstd -T0 -3' -cf "$out" sexos-v1.0.0.iso iso_root "${outputs[@]}"
echo "[ci.image] packed $(du -h "$out" | cut -f1): ISO, iso_root and ${#outputs[@]} target outputs"
