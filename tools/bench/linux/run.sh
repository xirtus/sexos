#!/usr/bin/env bash
# Boots a stock Linux kernel with bench.c as /init on the same emulated machine the
# SexOS gates use (q35, -cpu max, NVMe, 512 MiB), and prints its [bench] lines.
#   tools/bench/linux/run.sh /path/to/vmlinuz [cores] [dir with nvme-keyring.ko nvme-auth.ko nvme-core.ko nvme.ko]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
kernel="$1"; cores="${2:-1}"; mods="${3:-}"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
gcc -O2 -static -pthread -o "$work/init" "$here/bench.c"
mkdir -p "$work/root/dev" "$work/root/tmp"
cp "$work/init" "$work/root/init"
if [[ -n "$mods" ]]; then mkdir -p "$work/root/mods"; cp "$mods"/*.ko "$work/root/mods/"; fi
(cd "$work/root" && find . | cpio -o -H newc --quiet) > "$work/initrd"
truncate -s 1G "$work/disk.img"
accel=(); [[ "$cores" != 1 ]] && accel=(-accel tcg,thread=multi)
timeout 900 qemu-system-x86_64 -M q35 -cpu max,+pku -smp "$cores" "${accel[@]}" -m 512M \
  -kernel "$kernel" -initrd "$work/initrd" -append "console=ttyS0 quiet panic=-1" \
  -drive if=none,id=nvm,file="$work/disk.img",format=raw -device nvme,serial=bench,drive=nvm \
  -display none -serial stdio -no-reboot | grep -a '^\[bench'
