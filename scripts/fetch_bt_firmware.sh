#!/usr/bin/env bash
# Fetch the Intel Bluetooth firmware Cufflinks can ask for, from linux-firmware,
# pinned to the same commit as the Wi-Fi files and checked by SHA-256. Intel's
# redistributable blobs (LICENCE.ibt_firmware): data files the controller verifies
# and runs, never linked into SexOS.
#   ibt-19-0-4.{sfi,ddc}     AX201 (USB 8087:0026, HrP, legacy Read Version)
#   ibt-0041-0041.{sfi,ddc}  AX210 / Killer AX1675x (USB 8087:0032, the x17 R1)
#   ibt-0040-0041.{sfi,ddc}  AX211 / Killer AX1675i (USB 8087:0033)
# Usage: scripts/fetch_bt_firmware.sh [out-dir]   (default target/bt-firmware)
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/target/bt-firmware}"
COMMIT=69e51bb26f3fa3e118dd770f1962a93bed690dc1
BASE="https://gitlab.com/kernel-firmware/linux-firmware/-/raw/$COMMIT"
mkdir -p "$OUT"
while read -r sum name; do
  [ -n "$name" ] || continue
  if [ -f "$OUT/$name" ] && echo "$sum  $OUT/$name" | sha256sum -c --quiet - 2>/dev/null; then continue; fi
  for attempt in 1 2 3 4; do
    if curl -sSfL -o "$OUT/$name.part" "$BASE/intel/$name"; then break; fi
    [ "$attempt" = 4 ] && { echo "[bt.firmware] FAIL fetch $name" >&2; exit 1; }
    sleep $((attempt * 2))
  done
  echo "$sum  $OUT/$name.part" | sha256sum -c --quiet - || { echo "[bt.firmware] FAIL checksum $name" >&2; exit 1; }
  mv "$OUT/$name.part" "$OUT/$name"
done <<'LIST'
2e6a2027b9eeb3aa94255f10752bb828077739104b8be6769c7500322edddfd6 ibt-19-0-4.sfi
fe272982577efdc289cfe3e8bedafca607c0b7d6c9df1aab1880e22ef930d077 ibt-19-0-4.ddc
2e5f11ecc302e10dba51e9e07babef1b6d5c2a2359c695f1a23836552271d906 ibt-0041-0041.sfi
fe272982577efdc289cfe3e8bedafca607c0b7d6c9df1aab1880e22ef930d077 ibt-0041-0041.ddc
f14d059858623283f9e1bca3c9f5c5edc272cf538959d8f194327e3a9af5a75a ibt-0040-0041.sfi
fe272982577efdc289cfe3e8bedafca607c0b7d6c9df1aab1880e22ef930d077 ibt-0040-0041.ddc
LIST
curl -sSfL -o "$OUT/LICENCE.ibt_firmware" "$BASE/LICENSES/LICENCE.ibt_firmware" \
  || curl -sSfL -o "$OUT/LICENCE.ibt_firmware" "$BASE/LICENCE.ibt_firmware" \
  || echo "[bt.firmware] note: licence file not fetched"
echo "[bt.firmware] PASS dir=$OUT"
