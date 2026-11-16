#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""
Deterministic Mock ACE-Step HTTP Server.

Implements:
1. ace-server endpoints:
   - GET /props
   - POST /lm
   - POST /synth & POST /synth?wav=1
   - POST /understand
2. acestep-api endpoints:
   - GET /health
   - POST /release_task
   - POST /query_result
   - GET /v1/audio
"""

import argparse
import base64
import http.server
import io
import json
import math
import socketserver
import struct
import sys
import time
import urllib.parse
import uuid
import wave

# In-memory storage for asynchronous tasks
TASKS = {}


def generate_pcm_wav(duration: float = 1.0, bpm: float = 120.0, seed: int = 42) -> bytes:
    """Synthesize a deterministic valid 48 kHz 16-bit stereo RIFF WAV."""
    sample_rate = 48000
    channels = 2
    sampwidth = 2  # 16-bit signed PCM

    if duration <= 0:
        duration = 1.0
    duration = min(duration, 10.0)

    num_frames = int(sample_rate * duration)

    # Base frequencies derived deterministically from seed and bpm
    f_left = 220.0 + (float(seed) % 50.0) * 4.0
    f_right = 440.0 + (float(bpm) % 60.0) * 2.0

    buf = io.BytesIO()
    with wave.open(buf, "wb") as wf:
        wf.setnchannels(channels)
        wf.setsampwidth(sampwidth)
        wf.setframerate(sample_rate)
        wf.setnframes(num_frames)

        chunk_frames = 2048
        written = 0
        while written < num_frames:
            count = min(chunk_frames, num_frames - written)
            raw = bytearray()
            for i in range(count):
                idx = written + i
                t = idx / sample_rate
                # Smooth envelope attack and release to eliminate clicks
                env = min(1.0, idx / 2400.0, (num_frames - idx) / 2400.0)
                env = max(0.0, env)
                left = int(32767.0 * 0.4 * env * math.sin(2.0 * math.pi * f_left * t))
                right = int(32767.0 * 0.4 * env * math.sin(2.0 * math.pi * f_right * t))
                raw.extend(struct.pack("<hh", left, right))
            wf.writeframes(raw)
            written += count

    return buf.getvalue()


class MockAceHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        # Silence default request logging to keep test output clean
        pass

    def _send_json(self, status_code: int, data):
        body = json.dumps(data, indent=2).encode("utf-8")
        self.send_response(status_code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def _send_wav(self, wav_bytes: bytes):
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(wav_bytes)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        self.wfile.write(wav_bytes)

    def _read_body_json(self):
        content_length = int(self.headers.get("Content-Length", 0))
        if content_length > 0:
            body_bytes = self.rfile.read(content_length)
            try:
                return json.loads(body_bytes.decode("utf-8"))
            except Exception:
                return {}
        return {}

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)

        if path == "/props":
            self._send_json(
                200,
                {
                    "status": "ready",
                    "version": "1.5.0",
                    "model": "acestep-v1.5",
                    "sample_rate": 48000,
                    "channels": 2,
                    "capabilities": ["lm", "synth", "understand"],
                },
            )
        elif path == "/health":
            self._send_json(
                200,
                {
                    "code": 200,
                    "status": "healthy",
                    "service": "acestep-api",
                    "version": "1.5.0",
                },
            )
        elif path == "/v1/audio":
            task_path = query.get("path", [None])[0]
            duration = float(query.get("duration", [1.0])[0])
            bpm = float(query.get("bpm", [120.0])[0])
            seed = int(query.get("seed", [42])[0])

            if task_path:
                task_id = task_path.split("/")[-1].replace(".wav", "")
                if task_id in TASKS:
                    info = TASKS[task_id]
                    duration = info.get("duration", duration)
                    bpm = info.get("bpm", bpm)
                    seed = info.get("seed", seed)

            wav_data = generate_pcm_wav(duration=duration, bpm=bpm, seed=seed)
            self._send_wav(wav_data)
        else:
            self._send_json(404, {"error": "Not Found", "path": path})

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)
        data = self._read_body_json()

        if path == "/lm":
            prompt = data.get("prompt", "electro groove track")
            bpm = float(data.get("bpm", 128.0))
            key = data.get("key", "C major")
            seed = int(data.get("seed", 42))
            duration = float(data.get("duration", 30.0))
            lyrics = data.get(
                "lyrics",
                "[00:00.00]Synthesizing rhythm\n"
                "[00:06.00]Bassline rising up\n"
                "[00:12.00]Moan studio sound\n"
                "[00:18.00]Electric vibe drop",
            )
            enriched = [
                {
                    "prompt": prompt,
                    "bpm": bpm,
                    "key": key,
                    "time_signature": "4/4",
                    "duration": duration,
                    "lyrics": lyrics,
                    "seed": seed,
                }
            ]
            self._send_json(200, enriched)

        elif path == "/synth":
            duration = float(query.get("duration", [data.get("duration", 1.0)])[0])
            bpm = float(query.get("bpm", [data.get("bpm", 120.0)])[0])
            seed = int(query.get("seed", [data.get("seed", 42)])[0])
            wav_flag = query.get("wav", [str(data.get("wav", "0"))])[0]
            accept_header = self.headers.get("Accept", "")

            wav_bytes = generate_pcm_wav(duration=duration, bpm=bpm, seed=seed)

            # Return raw audio/wav if wav=1 or Accept: audio/wav is requested
            if wav_flag in ("1", "true", "yes") or "audio/wav" in accept_header or "audio/*" in accept_header:
                self._send_wav(wav_bytes)
            else:
                audio_b64 = base64.b64encode(wav_bytes).decode("ascii")
                self._send_json(
                    200,
                    {
                        "status": "ok",
                        "duration": duration,
                        "bpm": bpm,
                        "seed": seed,
                        "sample_rate": 48000,
                        "channels": 2,
                        "audio_base64": audio_b64,
                        "audio_url": f"/v1/audio?seed={seed}&bpm={bpm}&duration={duration}",
                    },
                )

        elif path == "/understand":
            self._send_json(
                200,
                {
                    "bpm": 124.0,
                    "key": "A minor",
                    "caption": "Electronic synthwave track with melodic lead and 808 percussion",
                    "lyrics": "[00:00.00]Intro beats\n[00:05.00]Melody unfolds\n[00:10.00]Harmonic climax",
                    "genre": "synthwave",
                    "confidence": 0.98,
                },
            )

        elif path == "/release_task":
            duration = float(data.get("duration", 1.0))
            bpm = float(data.get("bpm", 120.0))
            seed = int(data.get("seed", 42))
            task_id = f"task_{int(time.time() * 1000)}_{uuid.uuid4().hex[:6]}"
            TASKS[task_id] = {
                "status": 1,
                "duration": duration,
                "bpm": bpm,
                "seed": seed,
            }
            self._send_json(
                200,
                {
                    "code": 200,
                    "message": "Task created successfully",
                    "data": {
                        "task_id": task_id,
                    },
                },
            )

        elif path == "/query_result":
            task_id = data.get("task_id")
            if not task_id:
                if TASKS:
                    task_id = list(TASKS.keys())[-1]
                else:
                    task_id = "task_default"

            self._send_json(
                200,
                {
                    "code": 200,
                    "message": "success",
                    "data": [
                        {
                            "task_id": task_id,
                            "status": 1,
                            "audio_url": f"/v1/audio?path={task_id}.wav",
                        }
                    ],
                },
            )
        else:
            self._send_json(404, {"error": "Not Found", "path": path})


class ThreadedHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    parser = argparse.ArgumentParser(description="Deterministic Mock ACE-Step HTTP Server")
    parser.add_argument("--host", default="127.0.0.1", help="Host interface (default: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=8080, help="Port to listen on (0 for ephemeral, default: 8080)")
    args = parser.parse_args()

    server = ThreadedHTTPServer((args.host, args.port), MockAceHandler)
    bound_port = server.server_address[1]

    # Print port and ready message for scripts/tests to consume
    print(f"MOCK_ACE_SERVER_PORT={bound_port}", flush=True)
    print(f"Serving Mock ACE-Step HTTP Server on http://{args.host}:{bound_port}", flush=True)

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
