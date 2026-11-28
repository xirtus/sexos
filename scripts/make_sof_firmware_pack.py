#!/usr/bin/env python3
"""Packs the Intel-signed Sound Open Firmware images for Tiger Lake's audio DSP into the
one boot module BOOT hands Bell beside an Intel cAVS HD Audio lease (`sof.pack`,
sex_pdx::audio_dsp::FIRMWARE_MODULE). Format: crates/sex-wifi/src/pack.rs `SEXWIFW2`
with every file stored (inflated length 0), 4 KiB aligned, each with its SHA-256, so
`sex_wifi::pack::Pack::file("sof-tgl-h.ri")` reads it. Files come from
scripts/fetch_sof_firmware.sh (target/sof-firmware); without them the pack is empty and
Bell reports the DSP firmware as absent instead of the boot failing. Intel's licence
file goes in the pack beside the firmware it covers."""
import argparse, hashlib, struct, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAGIC = b"SEXWIFW2"
ENTRY = 88
NAME = 44
PAGE = 4096
IMAGES = ("sof-tgl-h.ri", "sof-tgl.ri")
LICENCE = "LICENCE.Intel"


def build(files):
    files = sorted(files, key=lambda f: f.name)
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


def collect_files(firmware: Path):
    """The images present, plus the licence; nothing when no image is there."""
    images = [firmware / n for n in IMAGES if (firmware / n).is_file()]
    if not images:
        return []
    licence = firmware / LICENCE
    return images + ([licence] if licence.is_file() else [])


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "target/sof-firmware")
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    files = collect_files(a.firmware)
    blob = build(files)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_bytes(blob)
    images = sum(1 for f in files if f.name in IMAGES)
    print(f"[sof.firmware.pack] files={len(files)} images={images} bytes={len(blob)} out={a.out}")


if __name__ == "__main__":
    sys.exit(main())
