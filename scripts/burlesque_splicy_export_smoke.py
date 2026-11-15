#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Host smoke gate for Burlesque rehearsed-timings WebM video export via Splicy:
1. Renders a 2-slide rehearsed presentation (intra-slide PopScale build + Magic Move)
   through `sex-splice`'s `Timeline` + `export_pictures` (`burlesque --dump-splicy-frames`).
2. Encodes those frames into a real VP9 `.webm` using Splicy's unmodified
   `crates/sex-media/src/export/bridge.c` (`sx_export_open(0, ...)` = `WebmVp9Opus`).
3. Verifies the output `.webm` with `ffprobe` and `ffmpeg` (VP9 codec, exact frame count,
   and animated pixel transitions across slides).
"""
import json
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, **kw).stdout


def main():
    out = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(tempfile.mkdtemp(prefix="burlesque-splicy-"))
    out.mkdir(parents=True, exist_ok=True)
    cleanup = len(sys.argv) <= 1

    try:
        raw_path = out / "frames.raw"
        webm_path = out / "deck.webm"
        smoke_bin = out / "burlesque_splicy_smoke"

        meta_raw = run(
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "--no-default-features",
                "-p",
                "burlesque",
                "--",
                "--dump-splicy-frames",
                str(raw_path),
            ],
            text=True,
        ).strip()
        meta = json.loads(meta_raw.splitlines()[-1])
        width = int(meta["width"])
        height = int(meta["height"])
        fps = int(meta["fps"])
        frames = int(meta["frames"])
        assert frames > 20, meta

        libs = shlex.split(
            run(
                ["pkg-config", "--cflags", "--libs", "libavcodec", "libavformat", "libavutil", "libswscale"],
                text=True,
            )
        )
        run(
            [
                "clang",
                "-std=c17",
                "-O2",
                "-Wall",
                "-Werror",
                "-D_GNU_SOURCE",
                "crates/sex-media/src/export/bridge.c",
                "scripts/burlesque_splicy_export_smoke.c",
                *libs,
                "-lm",
                "-o",
                str(smoke_bin),
            ]
        )

        run([str(smoke_bin), str(raw_path), str(webm_path)])
        webm_bytes = webm_path.stat().st_size
        assert webm_bytes > 500, webm_bytes

        probe = json.loads(
            run(
                [
                    "ffprobe",
                    "-v",
                    "error",
                    "-count_frames",
                    "-show_entries",
                    "stream=codec_name,codec_type,nb_read_frames,width,height,r_frame_rate:format=format_name,duration",
                    "-of",
                    "json",
                    str(webm_path),
                ],
                text=True,
            )
        )
        vstream = next(s for s in probe["streams"] if s["codec_type"] == "video")
        assert vstream["codec_name"] == "vp9", vstream
        assert int(vstream["width"]) == width, vstream
        assert int(vstream["height"]) == height, vstream
        assert int(vstream["nb_read_frames"]) == frames, (frames, vstream)
        assert vstream["r_frame_rate"] == f"{fps}/1", vstream

        rgb = run(
            [
                "ffmpeg",
                "-v",
                "error",
                "-i",
                str(webm_path),
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-",
            ]
        )
        frame_sz = width * height * 3
        assert len(rgb) == frames * frame_sz, (len(rgb), frames * frame_sz)

        first_frame = rgb[:frame_sz]
        last_frame = rgb[(frames - 1) * frame_sz :]
        assert any(b > 20 for b in first_frame), "first frame unexpectedly black"
        assert first_frame != last_frame, "Magic Move + builds should produce distinct start and end slide frames"

        print(
            json.dumps(
                {
                    "ok": True,
                    "encoder": "splicy:vp9/webm",
                    "bytes": webm_bytes,
                    "frames": frames,
                    "width": width,
                    "height": height,
                    "fps": fps,
                }
            )
        )
    finally:
        if cleanup:
            shutil.rmtree(out, ignore_errors=True)


if __name__ == "__main__":
    main()
