#!/usr/bin/env python3
"""Qualify actual guest FFV1/Matroska output with an independent host decode.

Four synthetic pictures exercise odd dimensions, frame ownership, delayed flush,
timestamp gaps, ordered file writes and container seek patches. No desktop or
audio capture is asserted. --extractor is the read-only SexFS extractor from the
desktop capture gate; its binary hash is retained.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from install_program import install
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("iso", "source-disk", "extractor", "out"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--program", type=Path, default=ROOT / "iso_root/programs/play")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    assert not any(args.out.iterdir()), "fresh evidence directory required"
    result = dict(ok=False, scope="actual SexOS FFmpeg synthetic encoder output; no desktop capture")
    for name in ("iso", "source_disk", "extractor", "program"):
        result[name + "_sha256"] = sha(getattr(args, name))
    result["gate_sha256"] = sha(Path(__file__))
    result["boot_helper_sha256"] = sha(ROOT / "scripts/quil_autosave_gate.py")
    disk = args.out / "nvme.img"
    shutil.copyfile(args.source_disk, disk)
    with disk.open("r+b") as output:
        output.truncate(1024**3)
    install(disk, [("play", args.program.read_bytes())])
    os.environ.setdefault("SEXOS_GATE_BOUND", "600")
    guest = None
    try:
        guest = boot(args.iso.resolve(), disk, args.out, "encode", "1280x800")
        guest["wait"](r'\[app.private.retired\] target=41 ', timeout=180)
        guest["focus"](153)
        time.sleep(.5)
        start = len(guest["text"]())
        for key in "run play --encode-proof proof.mkv":
            guest["q"].key({" ": "spc", "-": "minus", ".": "dot"}.get(key, key))
            time.sleep(.06)
        guest["q"].key("ret")
        made = guest["wait"](r'\[play.encode.proof\] synthetic=1 frames=4 width=65 height=49 fps=5 pts_ms=0,200,600,800 bytes=(\d+) synced=1\n', start, 240)
        result["guest_file_bytes"] = int(made[1])
        guest["q"].cmd("screendump", {"filename": str(args.out / "proof.ppm")})
        guest["stop"]()
        guest = None
        movie = args.out / "proof.mkv"
        subprocess.run([str(args.extractor.resolve()), str(disk), "/data/play/proof.mkv", str(movie)], check=True)
        assert movie.stat().st_size == result["guest_file_bytes"]
        probe = json.loads(subprocess.check_output(["ffprobe", "-v", "error", "-show_entries",
            "stream=codec_name,width,height,pix_fmt:packet=pts_time,duration_time", "-of", "json", str(movie)], text=True))
        (args.out / "probe.json").write_text(json.dumps(probe, indent=2) + "\n")
        stream, = probe["streams"]
        assert (stream["codec_name"], stream["width"], stream["height"], stream["pix_fmt"]) == ("ffv1", 65, 49, "bgr0")
        assert [float(p["pts_time"]) for p in probe["packets"]] == [0, .2, .6, .8]
        assert [float(p["duration_time"]) for p in probe["packets"]] == [.2] * 4
        decoded = args.out / "decoded.rgb"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(movie), "-fps_mode", "passthrough",
                        "-f", "rawvideo", "-pix_fmt", "rgb24", str(decoded)], check=True)
        expected = bytes(channel for frame in range(4) for y in range(49) for x in range(65)
                         for channel in ((x + frame * 17) % 256, (y * 3 + frame * 7) % 256, ((x ^ y) + frame * 33) % 256))
        (args.out / "expected.rgb").write_bytes(expected)
        assert decoded.read_bytes() == expected, "independent decode differs from synthetic source"
        result.update(ok=True, frames=4, decoded_bytes=len(expected), timestamps_ms=[0, 200, 600, 800],
                      movie_sha256=sha(movie), decoded_sha256=sha(decoded), serial_sha256=sha(args.out / "encode-serial.log"))
    except BaseException as error:
        result["error"] = repr(error)
        raise
    finally:
        if guest is not None:
            guest["stop"]()
        (args.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
