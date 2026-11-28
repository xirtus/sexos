#!/usr/bin/env bash
# Fetch the Intel-signed Sound Open Firmware images for Tiger Lake's audio DSP (the
# built-in microphones, docs/handoff/SOF_DMIC_PLAN_2026_11_28.md) from one pinned
# sof-bin release, checked by SHA-256: the tarball, then each file taken from it.
# SOF is BSD-3-Clause source; Intel's signed builds are redistributable
# (LICENCE.Intel, fetched beside them). Data files Bell loads into the DSP, never
# linked into SexOS.
# Usage: scripts/fetch_sof_firmware.sh [out-dir]   (default target/sof-firmware)
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/target/sof-firmware}"
VERSION=2026.09.2
TARBALL="sof-bin-$VERSION.tar.gz"
TARBALL_SHA256=5f7bc30b2b5c1e6b0da55a33fc44e2b7f56ae9b06952edcde614f51ef6b943e1
URL="https://github.com/thesofproject/sof-bin/releases/download/v$VERSION/$TARBALL"
# sha256, file name in OUT, path inside the tarball (IPC3, Intel-signed).
LIST='97c8d8dea6048329bfbb1dbbd56609934d388b7e3a9acffe20df0b7daf14d7d3 sof-tgl.ri sof/intel-signed/sof-tgl.ri
b9be9b8c542cb042b777438410aa169ed9b39bdc30c70edbfec41cd23ee80528 sof-tgl-h.ri sof/intel-signed/sof-tgl-h.ri
80169325f69cf8288ff225b590ca4bf2e8e2a15759076616fae0bfe2610427c9 LICENCE.Intel LICENCE.Intel'
mkdir -p "$OUT"

complete=1
while read -r sum name _; do
  [ -f "$OUT/$name" ] && echo "$sum  $OUT/$name" | sha256sum -c --quiet - 2>/dev/null || complete=0
done <<<"$LIST"
if [ "$complete" = 1 ]; then echo "[sof.firmware] PASS dir=$OUT cached=1"; exit 0; fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sof.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
for attempt in 1 2 3 4; do
  rc=0; curl -sSfL --connect-timeout 20 -o "$WORK/$TARBALL" "$URL" || rc=$?
  [ "$rc" = 0 ] && break
  # Host unreachable (resolve, connect or connect timeout): fail at once.
  case "$rc" in 6|7|28) echo "[firmware.fetch] FAIL host unreachable rc=$rc" >&2; exit 1;; esac
  [ "$attempt" = 4 ] && { echo "[sof.firmware] FAIL fetch $TARBALL" >&2; exit 1; }
  sleep $((attempt * 2))
done
echo "$TARBALL_SHA256  $WORK/$TARBALL" | sha256sum -c --quiet - || { echo "[sof.firmware] FAIL checksum $TARBALL" >&2; exit 1; }

members=()
while read -r _ _ path; do members+=("sof-bin-$VERSION/$path"); done <<<"$LIST"
mkdir "$WORK/x"
tar -xzf "$WORK/$TARBALL" -C "$WORK/x" --no-same-owner --no-same-permissions "${members[@]}" \
  || { echo "[sof.firmware] FAIL extract" >&2; exit 1; }
while read -r sum name path; do
  src="$WORK/x/sof-bin-$VERSION/$path"
  # Regular files only: never follow a link out of the archive.
  [ -f "$src" ] && [ ! -L "$src" ] || { echo "[sof.firmware] FAIL missing $path" >&2; exit 1; }
  echo "$sum  $src" | sha256sum -c --quiet - || { echo "[sof.firmware] FAIL checksum $name" >&2; exit 1; }
  cp "$src" "$OUT/$name.part" && mv "$OUT/$name.part" "$OUT/$name"
done <<<"$LIST"
echo "[sof.firmware] PASS dir=$OUT version=$VERSION"
