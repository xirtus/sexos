#!/usr/bin/env python3
"""Packs the Intel Wi-Fi firmware files into the one blob BOOT hands to the network
service (format: crates/sex-wifi/src/pack.rs, SEXWIFW2: large files raw DEFLATE so the
pack fits the installed disk's boot partition). Files come from
scripts/fetch_wifi_firmware.sh (target/wifi-firmware); without them the pack is
empty and the chip reports "firmware missing" instead of the boot failing."""
import argparse, hashlib, struct, sys, zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAGIC = b"SEXWIFW2"
ENTRY = 88
NAME = 44
# Wi-Fi files at least this big are stored compressed (the .ucode files, about 2.6 to 1).
COMPRESS_FROM = 64 * 1024
PAGE = 4096
MAX_FILES = 64


def build(files):
    files = sorted(files, key=lambda f: f.name)
    if len(files) > MAX_FILES:
        raise SystemExit(f"too many firmware files: {len(files)} > {MAX_FILES}")
    header = bytearray(MAGIC + struct.pack("<II", len(files), 0))
    data = bytearray()
    start = -(-(len(header) + ENTRY * len(files)) // PAGE) * PAGE
    for f in files:
        name = f.name.encode()
        if len(name) >= NAME:
            raise SystemExit(f"firmware name too long: {f.name}")
        raw = f.read_bytes()
        body, inflated = raw, 0
        if len(raw) >= COMPRESS_FROM and f.name.startswith("iwlwifi-"):  # Cufflinks reads ibt-* stored
            c = zlib.compressobj(9, zlib.DEFLATED, -15)
            packed = c.compress(raw) + c.flush()
            if len(packed) < len(raw):
                body, inflated = packed, len(raw)
        offset = start + len(data)
        header += name.ljust(NAME, b"\0") + struct.pack("<III", inflated, offset, len(body)) + hashlib.sha256(raw).digest()
        data += body
        data += b"\0" * (-len(data) % PAGE)
    header += b"\0" * (start - len(header))
    return bytes(header + data)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "target/wifi-firmware")
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    files = sorted(a.firmware.glob("iwlwifi-*")) if a.firmware.is_dir() else []
    files = [f for f in files if f.is_file() and not f.name.endswith(".part")]
    blob = build(files)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_bytes(blob)
    print(f"[wifi.firmware.pack] files={len(files)} bytes={len(blob)} out={a.out}")


if __name__ == "__main__":
    sys.exit(main())
