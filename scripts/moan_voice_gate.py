#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Native fixture bridge: POST, ranged WAV import, project save and cold reopen."""
import argparse
import http.server
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

from install_program import install
from moan_project_gate import Controls
from moan_gate_geometry import ready, control
from quil_autosave_gate import boot

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "crates/sex-voice/adapters"))
import mosslanding  # noqa: E402


class VoiceControls(Controls):
    def click(self, surface, name):
        x, y = control(self.g, surface, name.replace(" ", "_"), (self.rw, self.rh))
        self.move(x, y)
        self.button(True)
        self.button(False)


class ProgramServer(http.server.BaseHTTPRequestHandler):
    programs = {}
    def do_GET(self):
        body = self.programs.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *_):
        pass


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("iso", "source-disk", "moan", "out"):
        p.add_argument("--" + name, required=True, type=Path)
    p.add_argument("--resolution", default="1280x800")
    p.add_argument("--skip-fetch", action="store_true", help="source disk already has /bin/moan")
    p.add_argument("--skip-files-fetch", action="store_true", help="source disk already has /bin/files")
    a = p.parse_args()
    out = a.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    disk = out / "nvme.img"
    shutil.copyfile(a.source_disk, disk)
    with disk.open("r+b") as image:
        image.truncate(1 << 30)
    ProgramServer.programs = {
        "/moan": a.moan.read_bytes(),
        "/files": (ROOT / "target/x86_64-sex-app/release/files").read_bytes(),
    }
    program_server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ProgramServer)
    threading.Thread(target=program_server.serve_forever, daemon=True).start()
    program_port = program_server.server_address[1]
    os.environ["SEXOS_GATE_NIC"] = "-netdev user,id=n0 -device e1000e,netdev=n0"
    os.environ.setdefault("SEXOS_GATE_BOUND", "1200")
    adapter = mosslanding.Adapter(fixture=True)
    worker = threading.Thread(target=mosslanding.serve, args=(adapter, "127.0.0.1", 8875, ""), daemon=True)
    worker.start()
    time.sleep(.3)
    g = boot(a.iso, disk, out, "moan-voice", a.resolution)
    try:
        c = VoiceControls(g, out, a.resolution)
        g["wait"](r"\[app.private.retired\] target=41 ", 0, 180)
        if not a.skip_fetch:
            start = c.run(f"fetch http://10.0.2.2:{program_port}/moan /bin/moan")
            g["wait"](rf"\[spindle.fetch.line\] Saved {len(ProgramServer.programs['/moan'])} bytes", start, 300)
        start, target, task, surface = c.launch("moan", "run +net moan")
        g["wait"](r"\[moan.ready\]", start, 180)
        ready(g, surface)
        c.focus(surface)
        c.click(surface, "Voice")
        c.click(surface, "voice_prompt")
        c.type("Hello Moan")
        start = len(c.text())
        c.click(surface, "Generate Voice")
        queued = g["wait"](r"\[moan.voice.queued\] job=(\d+) endpoint=http://10.0.2.2:8875", start, 120)
        imported = g["wait"](rf"\[moan.voice.imported\] job={queued[1]} asset=(\d+) tracks=1 frames=4 model=fixture-v1", start, 180)
        assert int(imported[1]) == int(queued[1])
        saved = g["wait"](rf"\[moan.voice.snapshot\] job={queued[1]} path=/data/moan/voice-{queued[1]}\.moan bytes=(\d+) synced=1", start, 180)
        saved_bytes = int(saved[1])
        c.close(surface, target, task, dirty=True)
        c.q.cmd("quit")
        g["proc"].wait(30)
    finally:
        g["stop"]()
        program_server.shutdown()
    extractor = out / "sexfs-extract"
    subprocess.run(["rustc", "--edition=2021", "-O", "-A", "dead_code", "-A", "unused_attributes",
                    str(ROOT / "scripts/sexfs_extract.rs"), "-o", str(extractor)], check=True)
    project = out / "voice.moan"
    path = f"/data/moan/voice-{queued[1]}.moan"
    subprocess.run([str(extractor), str(disk), path, str(project)], check=True)
    data = project.read_bytes()
    assert len(data) == saved_bytes and data[:8] == b"MOANPRJ\0"
    assert struct.unpack_from("<H", data, 8)[0] == 5
    assert b"fixture-v1" in data
    g = boot(a.iso, disk, out, "moan-voice-reopen", a.resolution)
    try:
        c = VoiceControls(g, out, a.resolution)
        g["wait"](r"\[app.private.retired\] target=41 ", 0, 180)
        start, target, task, surface = c.launch("moan", f"run moan @{path}")
        g["wait"](rf"\[moan.project.opened\] bytes={saved_bytes} tracks=1 clips=1 hash=0x[0-9a-f]+ sources_shared=1 clean=1", start, 240)
        g["wait"](r"\[moan.ready\]", start, 180)
        ready(g, surface)
        c.close(surface, target, task)
        result = {"ok": True, "job": int(queued[1]), "asset": int(imported[1]),
                  "project_bytes": saved_bytes, "project_version": 5, "cold_reopen": True,
                  "worker_post": True, "ranged_wav": True}
        (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(result)
    finally:
        g["stop"]()
        adapter.close()


if __name__ == "__main__":
    main()
