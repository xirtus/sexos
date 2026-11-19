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
POOL="pool/universe/w/wine/libwine_${VERSION}_amd64.deb"
# Mirrors in order; GitHub's runners sit next to Azure's. The SHA-256 pins the content.
MIRRORS=(http://azure.archive.ubuntu.com/ubuntu http://archive.ubuntu.com/ubuntu https://mirrors.edge.kernel.org/ubuntu http://us.archive.ubuntu.com/ubuntu)
SHA256="eb421a3660bfdc090d70ce16b06aadfde1c2bce2593292023e520988f45d009e"
DLLS=(ntdll kernelbase kernel32 ucrtbase msvcrt)
# NLS tables for Wine's locale code (US English, code pages 1252 and 437).
NLS=(locale l_intl c_1252 c_437 c_20127 sortdefault normnfc normnfd)
DIR="${1:-${XDG_CACHE_HOME:-$HOME/.cache}/sexos/wine-9.0}"
if [[ -f "$DIR/.done-$SHA256" ]]; then echo "$DIR"; exit 0; fi
mkdir -p "$DIR"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
ok=0
for m in "${MIRRORS[@]}"; do
  # Bounded: a stalled mirror must not hang the gate (it once ran into the job timeout).
  if curl -fsSL --connect-timeout 20 --max-time 240 --speed-limit 50000 --speed-time 30 --retry 2 \
       -o "$T/libwine.deb" "$m/$POOL" >&2 \
     && echo "$SHA256  $T/libwine.deb" | sha256sum -c --quiet - >&2; then ok=1; break; fi
  echo "[fetch_wine_pe] $m failed, trying the next mirror" >&2
done
[[ $ok == 1 ]] || { echo "[fetch_wine_pe] FAIL no mirror served libwine $VERSION" >&2; exit 1; }
dpkg-deb -x "$T/libwine.deb" "$T" >&2
for d in "${DLLS[@]}"; do
  cp "$T/usr/lib/x86_64-linux-gnu/wine/x86_64-windows/$d.dll" "$DIR/$d.dll"
done
for n in "${NLS[@]}"; do
  cp "$T/usr/share/wine/nls/$n.nls" "$DIR/$n.nls"
done
touch "$DIR/.done-$SHA256"
echo "$DIR"
