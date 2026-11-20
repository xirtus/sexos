#!/usr/bin/env python3
"""Packs NVIDIA's GSP-RM firmware files into the one boot module BOOT hands to
sexgpu-nv (format: crates/sex-gpu-nv/src/pack.rs, SEXNVFW1, files stored raw on
4 KiB boundaries with their SHA-256). Files come from
scripts/fetch_nvidia_gsp_firmware.sh (target/nvidia-firmware); without them the pack
is empty and sexgpu-nv logs "firmware missing" instead of the boot failing. NVIDIA's
licence file goes in the pack beside the firmware it covers."""
import argparse, hashlib, struct, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAGIC = b"SEXNVFW1"
ENTRY = 88
NAME = 44
PAGE = 4096
FILES = ("booter_load-535.113.01.bin", "booter_unload-535.113.01.bin",
         "bootloader-535.113.01.bin", "gsp-535.113.01.bin", "LICENCE.nvidia")


def build(files):
    header = bytearray(MAGIC + struct.pack("<II", len(files), 0))
    data = bytearray()
    start = -(-(len(header) + ENTRY * len(files)) // PAGE) * PAGE
    for f in files:
        name = f.name.encode()
        if len(name) >= NAME:
            raise SystemExit(f"firmware name too long: {f.name}")
        raw = f.read_bytes()
        header += name.ljust(NAME, b"\0") + struct.pack("<III", 0, start + len(data), len(raw)) + hashlib.sha256(raw).digest()
        data += raw
        data += b"\0" * (-len(data) % PAGE)
    header += b"\0" * (start - len(header))
    return bytes(header + data)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "target/nvidia-firmware")
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    files = [a.firmware / n for n in FILES if (a.firmware / n).is_file()]
    # A partial set boots nothing: pack all four firmware files or none.
    if sum(1 for f in files if f.name.endswith(".bin")) != 4:
        files = []
    blob = build(files)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_bytes(blob)
    print(f"[nvidia.firmware.pack] files={len(files)} bytes={len(blob)} out={a.out}")


if __name__ == "__main__":
    sys.exit(main())
