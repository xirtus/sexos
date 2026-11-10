#!/usr/bin/env bash
# Fetch the Intel Wi-Fi firmware the driver can ask for, from linux-firmware,
# pinned to one commit and checked by SHA-256. These are Intel's redistributable
# blobs (LICENCE.iwlwifi_firmware): shipped as data files beside SexOS, loaded by
# the driver at start, never linked into it.
# Usage: scripts/fetch_wifi_firmware.sh [out-dir]   (default target/wifi-firmware)
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT_DIR/target/wifi-firmware}"
COMMIT=69e51bb26f3fa3e118dd770f1962a93bed690dc1
BASE="https://gitlab.com/kernel-firmware/linux-firmware/-/raw/$COMMIT"
mkdir -p "$OUT"
while read -r sum name; do
  [ -n "$name" ] || continue
  if [ -f "$OUT/$name" ] && echo "$sum  $OUT/$name" | sha256sum -c --quiet - 2>/dev/null; then continue; fi
  for attempt in 1 2 3 4; do
    if curl -sSfL -o "$OUT/$name.part" "$BASE/intel/iwlwifi/$name"; then break; fi
    [ "$attempt" = 4 ] && { echo "[wifi.firmware] FAIL fetch $name" >&2; exit 1; }
    sleep $((attempt * 2))
  done
  echo "$sum  $OUT/$name.part" | sha256sum -c --quiet - || { echo "[wifi.firmware] FAIL checksum $name" >&2; exit 1; }
  mv "$OUT/$name.part" "$OUT/$name"
done <<'LIST'
5e53c77a19f0cfabe039541bad814f90f47a6fec75ca6e3274610167c25897de iwlwifi-Qu-b0-hr-b0-77.ucode
71bfd1f64299dcc32a6b3ff068a3b7c189f56a78d9d90add4b79cd463506700e iwlwifi-Qu-c0-hr-b0-77.ucode
8f1b9bb98739a402d8414b7855b2c42256550ede329d06947d0f431f538dde77 iwlwifi-QuZ-a0-hr-b0-77.ucode
6b9013d1c9987042ad6a62e28a8e142c8a1f0348d783176a290be549d8902262 iwlwifi-QuZ-a0-jf-b0-77.ucode
19db22bb74427b2be3490ea3dcd19f1a4f1821233c9bfdec822be5a10cf8d335 iwlwifi-cc-a0-77.ucode
fd745741f4b9d722cfdc313747f04b4375c1e30370b97de59e2e9712e151822e iwlwifi-ma-b0-gf-a0-89.ucode
9d540dde17b8fce6bc08d39ac7745bafa724b6d49825c745aeb5650223424d98 iwlwifi-ma-b0-gf-a0.pnvm
87590ea9535a8939fddb296bc7622f3a900f28f47b45cb4e435a2a1d5f496ccd iwlwifi-so-a0-gf-a0-77.ucode
1597d11ad821a8e0c313c8bcac6201522407b10f60c78740bc057fac7f148d62 iwlwifi-so-a0-gf-a0.pnvm
d945bce539ee78033a85c288da9b988b2651a1623c5f6081b7db510d3967df76 iwlwifi-so-a0-gf4-a0-77.ucode
e55640e849b16a387895807c18fa3c73f6750aa4721eb7a1070739a316e5e20e iwlwifi-so-a0-gf4-a0.pnvm
fd8977dbfacfe50b8a6601479aa9ffd21d01e274e4a9efd13935dd9d667fd45b iwlwifi-ty-a0-gf-a0-77.ucode
9ec7f36114c68d518f6c34212c3e7959ff60be0d1fae8e774a70cf6d9b7159fd iwlwifi-ty-a0-gf-a0.pnvm
LIST
curl -sSfL -o "$OUT/LICENCE.iwlwifi_firmware" "https://gitlab.com/kernel-firmware/linux-firmware/-/raw/$COMMIT/LICENSES/LICENCE.iwlwifi_firmware"
echo "[wifi.firmware] PASS dir=$OUT"
