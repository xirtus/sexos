#!/usr/bin/env python3
"""Real encoder/output host smoke; explicitly not a guest or capture proof.

Requires system FFmpeg development libraries, pkg-config, clang, rustc, ffmpeg
and ffprobe. Retains compiler, packet, decoded-pixel and source provenance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    result = {"ok": False, "boundary": "system FFmpeg/libc; no SexOS guest or capture authority"}
    commands = []

    def run(command):
        commands.append(command)
        p = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
        with (out / "commands.log").open("a") as log:
            log.write(shlex.join(command) + "\n" + p.stdout + p.stderr)
        p.check_returncode()
        return p.stdout

    try:
        result["head"] = run(["git", "rev-parse", "HEAD"]).strip()
        result["dirty"] = run(["git", "status", "--porcelain"])
        sources = ["crates/sex-media/src/recording/bridge.c", "crates/sex-media/src/recording/mod.rs",
                   "crates/sex-media/src/recording/output.rs", "scripts/recording_host_smoke.rs",
                   "scripts/recording_host_gate.py"]
        result["sources"] = {name: sha(ROOT / name) for name in sources}
        result["clang"] = run([os.environ.get("CC", "clang"), "--version"])
        result["rustc"] = run(["rustc", "--version"])
        result["ffmpeg"] = run(["ffmpeg", "-version"])
        result["libraries"] = run(["pkg-config", "--modversion", "libavcodec", "libavformat", "libavutil"])
        flags = shlex.split(run(["pkg-config", "--cflags", "--libs", "libavcodec", "libavformat", "libavutil"]))
        library = "libsexos_record_bridge.dylib" if sys.platform == "darwin" else "libsexos_record_bridge.so"
        run([os.environ.get("CC", "clang"), "-std=c17", "-O2", "-Wall", "-Werror",
             *(["-dynamiclib"] if sys.platform == "darwin" else ["-shared", "-fPIC"]),
             "crates/sex-media/src/recording/bridge.c", *flags, "-o", str(out / library)])
        run(["rustc", "--edition=2021", "--cfg", "ffmpeg_recording", "scripts/recording_host_smoke.rs",
             "-L", "native=" + str(out), "-l", "dylib=sexos_record_bridge",
             "-C", "link-arg=-Wl,-rpath," + str(out), "-o", str(out / "smoke")])
        result["smoke"] = json.loads(run([str(out / "smoke"), str(out)]))
        probe = json.loads(run(["ffprobe", "-v", "error", "-show_entries",
                               "stream=codec_name,width,height,pix_fmt,color_range,color_space,color_transfer:packet=pts_time,duration_time",
                               "-of", "json", str(out / "proof.mkv")]))
        (out / "probe.json").write_text(json.dumps(probe, indent=2) + "\n")
        stream, = probe["streams"]
        assert (stream["codec_name"], stream["width"], stream["height"], stream["pix_fmt"]) == ("ffv1", 65, 49, "bgr0")
        assert [float(p["pts_time"]) for p in probe["packets"]] == [0, .2, .6, .8]
        assert [float(p["duration_time"]) for p in probe["packets"]] == [.2] * 4
        # Preserve actual packet frames across the intentional timestamp gap.
        # The CLI's default constant-rate conversion can duplicate a picture.
        run(["ffmpeg", "-v", "error", "-y", "-i", str(out / "proof.mkv"),
             "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", str(out / "decoded.rgb")])
        expected = (out / "expected.rgb").read_bytes()
        decoded = (out / "decoded.rgb").read_bytes()
        assert len(expected) == 65 * 49 * 3 * 4 and decoded == expected, "decoded RGB differs"
        result.update(ok=True, decoded_bytes=len(decoded), frames=4, timestamps_ms=[0, 200, 600, 800])
        result["artifacts"] = {name: sha(out / name) for name in [library, "smoke", "proof.mkv", "expected.rgb", "decoded.rgb", "probe.json"]}
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        result["commands"] = commands
        (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"ok": True, "result": str(out / "result.json")}))


if __name__ == "__main__":
    main()
