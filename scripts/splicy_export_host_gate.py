#!/usr/bin/env python3
"""Splicy export on the host: each format (WebM VP9+Opus, MP4 H.264+AAC, MP4 AV1+Opus)
written through the export bridge's callbacks, then probed and decoded by system FFmpeg.

Checks codecs, 60 frames at 30 fps, two seconds of sound, and that the cut (red to blue
at frame 30) lands on exactly that frame. Not a guest proof: system FFmpeg and libc.
Needs clang, pkg-config, FFmpeg development libraries with libvpx, libx264 and libaom
(or SVT-AV1), ffmpeg and ffprobe.
"""
import json
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORMATS = [(0, "webm", "vp9", "opus"), (1, "mp4", "h264", "aac"), (2, "mp4", "av1", "opus")]


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, **kw).stdout


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(tempfile.mkdtemp())
    out.mkdir(parents=True, exist_ok=True)
    libs = shlex.split(run(["pkg-config", "--cflags", "--libs", "libavcodec", "libavformat", "libavutil", "libswscale"], text=True))
    smoke = out / "smoke"
    run(["clang", "-std=c17", "-O2", "-Wall", "-Werror", "-D_GNU_SOURCE", "crates/sex-media/src/export/bridge.c",
         "scripts/splicy_export_host_smoke.c", *libs, "-lm", "-o", str(smoke)])
    report = {}
    for fmt, ext, video, audio in FORMATS:
        path = out / f"export{fmt}.{ext}"
        run([str(smoke), str(fmt), str(path)])
        probe = json.loads(run(["ffprobe", "-v", "error", "-count_frames", "-show_entries",
                                "stream=codec_name,codec_type,nb_read_frames,r_frame_rate:format=duration,format_name",
                                "-of", "json", str(path)], text=True))
        streams = {s["codec_type"]: s for s in probe["streams"]}
        assert streams["video"]["codec_name"] == video, streams
        assert streams["audio"]["codec_name"] == audio, streams
        assert int(streams["video"]["nb_read_frames"]) == 60, streams
        assert streams["video"]["r_frame_rate"] == "30/1", streams
        assert abs(float(probe["format"]["duration"]) - 2.0) < 0.1, probe["format"]
        rgb = run(["ffmpeg", "-v", "error", "-i", str(path), "-fps_mode", "passthrough",
                   "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        size = 64 * 48 * 3
        assert len(rgb) == 60 * size, len(rgb)

        def mean(n):
            f = rgb[n * size:(n + 1) * size]
            return [sum(f[c::3]) // (64 * 48) for c in range(3)]
        before, after = mean(29), mean(30)
        assert before[0] > 200 and before[2] < 60, (fmt, before)
        assert after[2] > 200 and after[0] < 60, (fmt, after)
        report[ext + ":" + video] = {"bytes": path.stat().st_size, "frame29": before, "frame30": after}
    print(json.dumps({"ok": True, "formats": report}))


if __name__ == "__main__":
    main()
