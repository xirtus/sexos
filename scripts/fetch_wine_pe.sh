#!/usr/bin/env bash
# Fetches Wine's 64-bit PE DLLs (Crossdress CW3) into a cache folder and prints it.
#
# Pinned: Wine 9.0 as Ubuntu 24.04 ships it (libwine 9.0~repack-4build3, LGPL-2.1+).
# Its source is the matching Ubuntu source package (`apt-get source wine=9.0~repack-4build3`),
# listed in docs/BRIDGE_LEDGER.md's foreign-code table. Only the PE halves are used: their
# Unix halves (ntdll.so and the rest) are replaced by Crossdress's host side.
#
# Usage: scripts/fetch_wine_pe.sh [DIR]   (default: ~/.cache/sexos/wine-9.0)
set -euo pipefail
VERSION="9.0~repack-4build3"
URL="http://archive.ubuntu.com/ubuntu/pool/universe/w/wine/libwine_${VERSION}_amd64.deb"
SHA256="eb421a3660bfdc090d70ce16b06aadfde1c2bce2593292023e520988f45d009e"
DLLS=(ntdll kernelbase kernel32 ucrtbase msvcrt)
DIR="${1:-${XDG_CACHE_HOME:-$HOME/.cache}/sexos/wine-9.0}"
if [[ -f "$DIR/.done-$SHA256" ]]; then echo "$DIR"; exit 0; fi
mkdir -p "$DIR"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
curl -fsSL --retry 4 -o "$T/libwine.deb" "$URL" >&2
echo "$SHA256  $T/libwine.deb" | sha256sum -c --quiet - >&2
dpkg-deb -x "$T/libwine.deb" "$T" >&2
for d in "${DLLS[@]}"; do
  cp "$T/usr/lib/x86_64-linux-gnu/wine/x86_64-windows/$d.dll" "$DIR/$d.dll"
done
touch "$DIR/.done-$SHA256"
echo "$DIR"
