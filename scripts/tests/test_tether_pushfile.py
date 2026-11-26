# SPDX-License-Identifier: GPL-3.0-only
"""Host unit tests for scripts/tether_host.py `push-file` binary framing and CLI staging."""
import json
from pathlib import Path
import struct
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import tether_host


def make_pcm_wav(sample_rate: int, channels: int, bits: int, pcm: bytes) -> bytes:
    block_align = channels * (bits // 8)
    byte_rate = sample_rate * block_align
    fmt = struct.pack("<4sIHHIIHH", b"fmt ", 16, 1, channels, sample_rate, byte_rate, block_align, bits)
    data = struct.pack("<4sI", b"data", len(pcm)) + pcm
    body = b"WAVE" + fmt + data
    return struct.pack("<4sI", b"RIFF", len(body)) + body


class TetherPushfileTests(unittest.TestCase):
    def test_build_pushfile_frame_roundtrip_wav(self):
        wav = make_pcm_wav(24000, 1, 16, struct.pack("<4h", 0, 1000, 2000, -2000))
        frame = tether_host.build_pushfile_frame("/media/Ventoy/moss.wav", wav)
        header, payload = frame.split(b"\n", 1)
        parts = header.decode("utf-8").split()
        self.assertEqual(parts[0], "SEXOS-PUSHFILE")
        self.assertEqual(parts[1], "/media/Ventoy/moss.wav")
        self.assertEqual(int(parts[2]), len(wav))
        self.assertEqual(int(parts[3], 16), tether_host.fnv64(wav))
        self.assertEqual(payload, wav)

    def test_build_pushfile_frame_empty_file(self):
        frame = tether_host.build_pushfile_frame("/tmp/empty.json", b"")
        header, payload = frame.split(b"\n", 1)
        self.assertEqual(header, b"SEXOS-PUSHFILE /tmp/empty.json 0 cbf29ce484222325")
        self.assertEqual(payload, b"")

    def test_build_pushfile_frame_rejects_bad_inputs(self):
        with self.assertRaises(ValueError):
            tether_host.build_pushfile_frame("", b"abc")
        with self.assertRaises(ValueError):
            tether_host.build_pushfile_frame("/path with space.wav", b"abc")
        with self.assertRaises(ValueError):
            tether_host.build_pushfile_frame("/bad\npath.wav", b"abc")
        with self.assertRaises(ValueError):
            tether_host.build_pushfile_frame("/" + "a" * tether_host.PUSHFILE_MAX_PATH, b"abc")
        with self.assertRaises(ValueError):
            tether_host.build_pushfile_frame("/too/big.bin", b"\x00" * (tether_host.PUSHFILE_MAX_BYTES + 1))

    def test_push_file_cli_stages_frame_and_reads_ok_reply(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            src = d / "tailor-keys.json"
            src.write_bytes(b'{"deepseek":"test"}\n')
            log = d / "session.log"
            log.write_bytes(b"--- spindle ---\n")
            (d / "current.log").symlink_to(log.name)
            (d / "state.json").write_text(json.dumps({"connected": True}))

            def fake_spindle():
                time.sleep(0.08)
                with log.open("ab") as f:
                    f.write(b"push-file: ok /tailor-keys.json (20 bytes)\n")

            t = threading.Thread(target=fake_spindle, daemon=True)
            t.start()
            rc = tether_host.push_file(d, src, "/tailor-keys.json", wait=2.0)
            t.join()
            self.assertEqual(rc, 0)
            inbox = (d / "inbox.txt").read_text().strip()
            self.assertTrue(inbox.startswith("@PUSHFILE pushfile-"))
            staged_name = inbox.split()[1]
            staged = d / staged_name
            self.assertTrue(staged.exists())
            self.assertEqual(staged.read_bytes(), tether_host.build_pushfile_frame("/tailor-keys.json", src.read_bytes()))


if __name__ == "__main__":
    unittest.main()
