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


def collect_files(wifi_dir: Path, bt_dir: Path):
    by_name = {}
    if wifi_dir.is_dir():
        for pattern in ("iwlwifi-*", "ibt-*"):
            for f in sorted(wifi_dir.glob(pattern)):
                if f.is_file() and not f.name.endswith(".part"):
                    by_name[f.name] = f
    if bt_dir.is_dir():
        for f in sorted(bt_dir.glob("ibt-*")):
            if f.is_file() and not f.name.endswith(".part"):
                by_name[f.name] = f
    return [by_name[k] for k in sorted(by_name)]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "target/wifi-firmware")
    p.add_argument("--bt-firmware", type=Path, default=ROOT / "target/bt-firmware")
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    files = collect_files(a.firmware, a.bt_firmware)
    blob = build(files)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_bytes(blob)
    wifi_count = sum(1 for f in files if f.name.startswith("iwlwifi-"))
    bt_count = sum(1 for f in files if f.name.startswith("ibt-"))
    print(f"[wifi.firmware.pack] files={len(files)} wifi={wifi_count} bt={bt_count} bytes={len(blob)} out={a.out}")


if __name__ == "__main__":
    sys.exit(main())
