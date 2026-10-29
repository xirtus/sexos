#!/usr/bin/env python3
"""Host evidence checks used by disk_persistence_gate.sh; no VM operations."""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re


@dataclass
class ProofRow:
    name: str
    passed: bool
    detail: str

    def render(self) -> str:
        return f"ROW {self.name} {'PASS' if self.passed else 'FAIL'} {self.detail}"


@dataclass
class DocumentProof:
    length: int
    fingerprint: int
    document: int


def document_markers(log: str, operation: str) -> list[DocumentProof]:
    proofs = []
    marker = f"[quil.persist.{operation}.ok]"
    for line in log.splitlines():
        if marker not in line:
            continue
        fields = {}
        for token in line.split(marker, 1)[1].split():
            if "=" not in token:
                continue
            key, value = token.split("=", 1)
            if key in fields:
                raise ValueError(f"duplicate_{operation}_{key}")
            fields[key] = value
        if not re.fullmatch(r"[0-9]+", fields.get("bytes", "")):
            raise ValueError(f"missing_or_invalid_{operation}_bytes")
        if not re.fullmatch(r"[0-9]+", fields.get("doc", "")):
            raise ValueError(f"missing_or_invalid_{operation}_doc")
        if not re.fullmatch(r"0x[0-9a-fA-F]{1,16}", fields.get("hash", "")):
            raise ValueError(f"missing_or_invalid_{operation}_hash")
        proof = DocumentProof(int(fields["bytes"]), int(fields["hash"], 16), int(fields["doc"]))
        if not 0 < proof.length <= 0xFFFF_FFFF or not 0 <= proof.document < (1 << 64) - 1:
            raise ValueError(f"out_of_range_{operation}_metadata")
        proofs.append(proof)
    if not proofs:
        raise ValueError(f"missing_{operation}_success")
    return proofs


def persistence_rows(boot1: str, boot2: str) -> list[ProofRow]:
    try:
        saved = document_markers(boot1, "save")[-1]
        loads = document_markers(boot2, "load")
        matching = [proof for proof in loads if proof.document == saved.document]
        # The gate triggers both palette Load and Linen disk-document Open.
        enough = len(matching) >= 2
        lengths_ok = enough and all(proof.length == saved.length for proof in matching)
        hashes_ok = lengths_ok and all(proof.fingerprint == saved.fingerprint for proof in matching)
        detail = f"doc={saved.document} save={saved.length} loads={len(matching)}"
        return [ProofRow("persist_bytes_match", lengths_ok, detail),
                ProofRow("persist_fingerprint_match", hashes_ok,
                         f"{detail} hash={saved.fingerprint:#x}")]
    except ValueError as error:
        return [ProofRow(name, False, f"reason={error}")
                for name in ["persist_bytes_match", "persist_fingerprint_match"]]


def ppm_rgb(data: bytes, min_width: int = 1272, min_height: int = 360) -> tuple[int, int, bytes]:
    # Read P6 tokens, allowing standard comments before/between header tokens.
    # After maxval consume precisely its separator: raster bytes may themselves
    # contain whitespace and must never be stripped or tokenized.
    position = 0
    tokens = []
    for _ in range(4):
        while position < len(data):
            if data[position] in b" \t\r\n\v\f":
                position += 1
            elif data[position] == ord("#"):
                newline = data.find(b"\n", position)
                if newline < 0:
                    raise ValueError("incomplete_ppm_comment")
                position = newline + 1
            else:
                break
        start = position
        while position < len(data) and data[position] not in b" \t\r\n\v\f":
            position += 1
        if start == position:
            raise ValueError("incomplete_ppm_header")
        tokens.append(data[start:position])
    if tokens[0] != b"P6":
        raise ValueError("unsupported_ppm_format")
    if not all(re.fullmatch(rb"[0-9]+", token) for token in tokens[1:]):
        raise ValueError("malformed_ppm_dimensions")
    width, height, maximum = map(int, tokens[1:])
    if width < min_width or height < min_height or maximum != 255:
        raise ValueError("ppm_bounds_or_depth")
    if position >= len(data):
        raise ValueError("missing_ppm_raster_separator")
    position += 2 if data[position:position + 2] == b"\r\n" else 1
    pixels = data[position:]
    if len(pixels) != width * height * 3:
        raise ValueError("ppm_raster_length")
    return width, height, pixels


def pixel_row(path: Path, log: str | None = None) -> ProofRow:
    """Measure Quil text-color occupancy, not glyphs or document identity.

    The separate transport length/fingerprint checks establish restored data.
    A colored stripe can satisfy this deliberately limited visual check.
    """
    try:
        width, height, pixels = ppm_rgb(path.read_bytes())
        if log is None:
            # Historical evidence remains readable; current gates supply a log.
            x0,y0,x1,y1=1072,56,1272,360
        else:
            views=re.findall(r'\[shell\.app\.viewport\] target=9 sid=156 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',log)
            if not views:raise ValueError("missing_quil_viewport")
            x,y,w,h=map(int,views[-1]);x0,y0=max(0,x),max(50,y);x1,y1=min(width,x+w),min(height,y+h)
            if w<=0 or h<=0 or x0>=x1 or y0>=y1:raise ValueError("invalid_quil_viewport")
        count = sum(pixels[(y * width + x) * 3:(y * width + x) * 3 + 3] == b"\xe0\xf0\xff"
                    for y in range(y0,y1) for x in range(x0,x1))
        return ProofRow("pixel_quil_text_color_occupancy", count > 50,
                        f"count={count} bounds={x0},{y0},{x1-x0},{y1-y0} glyph_identity=unverified document_identity=transport_fingerprint")
    except (OSError, ValueError) as error:
        reason = str(error).replace(" ", "_")
        return ProofRow("pixel_quil_text_color_occupancy", False, f"reason={reason}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    logs = sub.add_parser("logs")
    logs.add_argument("boot1", type=Path)
    logs.add_argument("boot2", type=Path)
    pixels = sub.add_parser("pixels")
    pixels.add_argument("screenshot", type=Path)
    pixels.add_argument("--log", type=Path, help="current native Quil viewport intent")
    args = parser.parse_args()
    if args.mode == "pixels":
        try:
            log=args.log.read_text(errors="replace") if args.log is not None else None
            rows = [pixel_row(args.screenshot,log)]
        except OSError as exc:
            rows=[ProofRow("pixel_quil_text_color_occupancy",False,f"reason={exc}")]
    else:
        try:
            rows = persistence_rows(args.boot1.read_text(errors="replace"), args.boot2.read_text(errors="replace"))
        except OSError as error:
            rows = [ProofRow("persist_fingerprint_match", False, f"reason={error}")]
    for row in rows:
        print(row.render())
    return 0 if all(row.passed for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
