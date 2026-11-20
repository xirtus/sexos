#!/usr/bin/env bash
# Fetch NVIDIA's GSP-RM firmware for Ampere (GA102..GA107, so the x17 R1's GA104)
# from linux-firmware, pinned to one commit and checked by SHA-256. These are
# NVIDIA's redistributable blobs (LICENSES/LICENCE.nvidia): shipped as data files
# beside SexOS and loaded into the GPU by sexgpu-nv, never linked into it.
# linux-firmware links nvidia/ga104/gsp -> ga102/gsp, so the ga102 files serve all.
# Usage: scripts/fetch_nvidia_gsp_firmware.sh [out-dir]   (default target/nvidia-firmware)
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/target/nvidia-firmware}"
COMMIT=69e51bb26f3fa3e118dd770f1962a93bed690dc1
BASE="https://gitlab.com/kernel-firmware/linux-firmware/-/raw/$COMMIT"
mkdir -p "$OUT"
while read -r sum name; do
  [ -n "$name" ] || continue
  if [ -f "$OUT/$name" ] && echo "$sum  $OUT/$name" | sha256sum -c --quiet - 2>/dev/null; then continue; fi
  for attempt in 1 2 3 4; do
    rc=0; curl -sSfL --connect-timeout 20 -o "$OUT/$name.part" "$BASE/nvidia/ga102/gsp/$name" || rc=$?
    [ "$rc" = 0 ] && break
    # Host unreachable (resolve, connect or connect timeout): fail at once, not after
    # every file and attempt; the host gates then skip the real-image tests.
    case "$rc" in 6|7|28) echo "[firmware.fetch] FAIL host unreachable rc=$rc" >&2; exit 1;; esac
    [ "$attempt" = 4 ] && { echo "[nvidia.firmware] FAIL fetch $name" >&2; exit 1; }
    sleep $((attempt * 2))
  done
  echo "$sum  $OUT/$name.part" | sha256sum -c --quiet - || { echo "[nvidia.firmware] FAIL checksum $name" >&2; exit 1; }
  mv "$OUT/$name.part" "$OUT/$name"
done <<'LIST'
3ab46bf4c70d72e8fc89d98e6bdc9b83a61954a586629d26b8b5653a620237e0 booter_load-535.113.01.bin
09240c83595856a0fe70d2ab3ec7325455cd8adfc48ec7b47f6fced1e5431beb booter_unload-535.113.01.bin
076d2a0675131ad8a8de215c611ce4271289adcc03e2cf39995f9b29079e2136 bootloader-535.113.01.bin
e30231ec0d317021770e16c81ae2c0553f728d0344248827582f78132c9447d4 gsp-535.113.01.bin
LIST
[ -f "$OUT/LICENCE.nvidia" ] || curl -sSfL --connect-timeout 20 -o "$OUT/LICENCE.nvidia" "$BASE/LICENSES/LICENCE.nvidia"
echo "[nvidia.firmware] PASS dir=$OUT"
