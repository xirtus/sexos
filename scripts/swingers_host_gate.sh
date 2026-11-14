#!/usr/bin/env bash
# Swingers against a real BitTorrent client (aria2) on the host:
#  1. aria2 seeds a torrent Swingers made; Swingers downloads it (file hash equal).
#  2. The same through a magnet link (metadata from aria2 over ut_metadata).
#  3. Swingers seeds; aria2 finds it through a local tracker and downloads it.
#  4. The daemon takes the magnet over Transmission's RPC (transmission-remote), downloads
#     it from that seeder, and lists, stops and removes it with its data.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
W=$(mktemp -d); trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$W"' EXIT
cargo build -q --locked -p swingers
B="$ROOT/target/debug/swingers"
cd "$W"; mkdir -p seed
head -c 12000000 /dev/urandom > seed/film.bin
want=$(sha256sum seed/film.bin | cut -c1-64)
python3 "$ROOT/scripts/swingers_tracker.py" --port 16970 > tracker.log 2>&1 &
sleep 1
"$B" create seed/film.bin --out film.torrent --v1 --tracker http://127.0.0.1:16970/announce | tee create.log
hash=$(grep -o 'btih%3A[0-9a-f]*' create.log | cut -d A -f2)
aria=(aria2c -q --enable-dht=false --bt-enable-lpd=false --enable-peer-exchange=false --seed-ratio=0)
check() { got=$(grep -a "^\[swingers.file\]" "$1" | awk '{print $3}'); [[ "$got" == "$want" ]] || { echo "FAIL $2: $got != $want"; tail -20 "$1"; exit 1; }; echo "ok $2"; }

"${aria[@]}" --listen-port=16881 -d seed -V --seed-time=2 film.torrent > aria1.log 2>&1 &
sleep 2
timeout 120 "$B" get film.torrent --dir t1 --peer 127.0.0.1:16881 --port 16891 > t1.log
check t1.log "download from aria2 (.torrent)"
timeout 120 "$B" get "magnet:?xt=urn:btih:$hash&x.pe=127.0.0.1:16881" --dir t2 --port 16892 > t2.log
check t2.log "download from aria2 (magnet)"

timeout 150 "$B" get film.torrent --dir t1 --port 16893 --seed-secs 120 > t3.log &
sleep 3
timeout 120 "${aria[@]}" --seed-time=0 --listen-port=16882 -d t4 film.torrent
got=$(sha256sum t4/film.bin | cut -c1-64); [[ "$got" == "$want" ]] || { echo "FAIL aria2 from Swingers"; exit 1; }
echo "ok aria2 downloads from Swingers"

"$B" daemon --dir t5 --port 16894 --rpc 16991 --user u --password p > t5.log 2>&1 &
sleep 1
R=(transmission-remote 127.0.0.1:16991 -n u:p)
"${R[@]}" -a "magnet:?xt=urn:btih:$hash&x.pe=127.0.0.1:16893" | grep -q success
for _ in $(seq 120); do "${R[@]}" -l | grep -q " 100% " && break; sleep 1; done
"${R[@]}" -l
got=$(sha256sum t5/film.bin | cut -c1-64); [[ "$got" == "$want" ]] || { echo "FAIL daemon over RPC"; tail -20 t5.log; exit 1; }
"${R[@]}" -t 1 -S | grep -q success
"${R[@]}" -t 1 --remove-and-delete | grep -q success
[[ ! -e t5/film.bin ]] || { echo "FAIL remove-and-delete left the file"; exit 1; }
noauth=$(transmission-remote 127.0.0.1:16991 -l 2>&1 || true); [[ "$noauth" == *401* ]] || { echo "FAIL RPC without password was let in"; exit 1; }
echo "ok daemon over Transmission RPC"
echo "swingers host gate passed"
