"""Local bridge from the SexOS voice frame v1 to MOSSlanding's HTTP backend.

Run: python3 mosslanding.py --fixture
     python3 mosslanding.py --backend http://127.0.0.1:8765

POST /v1/frame with application/octet-stream SXVC frames. A Submit returns Queued;
GetStatus returns Status until it can return Result. GET /v1/assets/<id> returns WAV.
The existing MOSSlanding backend remains responsible for model loading/inference.
This adapter never runs model weights and binds to loopback by default.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import io
import json
import os
import struct
import threading
import time
import urllib.error
import urllib.request
import wave
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MAGIC = b"SXVC"
VERSION = 1
MAX_FRAME = 4096
MAX_TEXT = 3072
MAX_MODEL = 96
MAX_LANGUAGE = 24
FIXTURE_WAV = bytes([
    82, 73, 70, 70, 44, 0, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32,
    16, 0, 0, 0, 1, 0, 1, 0, 64, 31, 0, 0, 128, 62, 0, 0,
    2, 0, 16, 0, 100, 97, 116, 97, 8, 0, 0, 0, 0, 0, 232, 3, 24, 252, 0, 0,
])


class ProtocolError(ValueError):
    pass


def decode(frame: bytes) -> dict:
    if len(frame) < 16 or len(frame) > MAX_FRAME:
        raise ProtocolError("frame size")
    if frame[:4] != MAGIC or frame[4] != VERSION or frame[6:8] != b"\0\0":
        raise ProtocolError("frame header")
    kind, job_id = frame[5], struct.unpack_from("<Q", frame, 8)[0]
    if not job_id:
        raise ProtocolError("job id")
    if kind in (2, 3):
        if len(frame) != 16:
            raise ProtocolError("trailing control bytes")
        return {"kind": kind, "job_id": job_id}
    if kind != 1 or len(frame) < 52:
        raise ProtocolError("kind or submit size")
    operation = frame[16]
    if operation not in (1, 2, 3) or frame[17:20] != b"\0\0\0":
        raise ProtocolError("operation")
    seed, voice_id, reference_id, max_frames = struct.unpack_from("<QQQI", frame, 20)
    language_len, model_len, text_len = struct.unpack_from("<BBH", frame, 48)
    if language_len > MAX_LANGUAGE or model_len > MAX_MODEL or text_len > MAX_TEXT:
        raise ProtocolError("field bound")
    if len(frame) != 52 + language_len + model_len + text_len:
        raise ProtocolError("payload size")
    at = 52
    try:
        language = frame[at:at + language_len].decode("utf-8")
        at += language_len
        model = frame[at:at + model_len].decode("utf-8")
        at += model_len
        text = frame[at:at + text_len].decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ProtocolError("utf-8") from exc
    if not model or not max_frames or (operation in (2, 3) and not text) or (operation in (1, 3) and not reference_id):
        raise ProtocolError("missing required field")
    return {"kind": kind, "job_id": job_id, "operation": operation, "seed": seed,
            "voice_id": voice_id, "reference_id": reference_id, "max_frames": max_frames,
            "language": language, "model": model, "text": text}


def header(kind: int, job_id: int) -> bytes:
    return MAGIC + bytes([VERSION, kind, 0, 0]) + struct.pack("<Q", job_id)


def status(job_id: int, state: int, progress: int = 0, error: int = 0) -> bytes:
    return header(4, job_id) + struct.pack("<BBHH", state, 0, progress, error)


def result(job_id: int, asset_id: int, wav: bytes, model: str, seed: int) -> bytes:
    try:
        with wave.open(io.BytesIO(wav), "rb") as audio:
            channels, sample_rate, frames = audio.getnchannels(), audio.getframerate(), audio.getnframes()
            if audio.getcomptype() != "NONE" or audio.getsampwidth() != 2:
                raise ProtocolError("worker did not return 16-bit PCM WAV")
    except (EOFError, wave.Error) as exc:
        raise ProtocolError("worker did not return WAV") from exc
    model_bytes = model.encode("utf-8")
    if not 1 <= channels <= 8 or not 8_000 <= sample_rate <= 192_000 or not frames or len(model_bytes) > MAX_MODEL:
        raise ProtocolError("invalid output metadata")
    return (header(5, job_id) + struct.pack("<QIB3xQ", asset_id, sample_rate, channels, frames)
            + hashlib.sha256(wav).digest() + struct.pack("<QB", seed, len(model_bytes)) + model_bytes)


@dataclass
class Job:
    request: dict
    state: int = 1  # Queued
    progress: int = 0
    error: int = 0
    asset_id: int = 0
    model: str = "fixture-v1"
    fixture_polls: int = 0


class Adapter:
    def __init__(self, fixture: bool, backend: str = "http://127.0.0.1:8765", voices: dict[int, str] | None = None):
        self.fixture = fixture
        self.backend = backend.rstrip("/")
        self.voices = voices or {}
        self.jobs: dict[int, Job] = {}
        self.assets: dict[int, bytes] = {}
        self.lock = threading.Lock()
        self.pool = ThreadPoolExecutor(max_workers=1)

    def close(self):
        self.pool.shutdown(wait=True)

    def handle(self, frame: bytes) -> bytes:
        req = decode(frame)
        job_id = req["job_id"]
        with self.lock:
            if req["kind"] == 1:
                if job_id in self.jobs:
                    raise ProtocolError("duplicate job id")
                if req["operation"] != 2:
                    raise ProtocolError("MOSSlanding HTTP backend currently supports synthesis only")
                if not self.fixture:
                    if req["seed"] != 0:
                        raise ProtocolError("current MOSSlanding backend cannot honor seed")
                    if req["model"] != "active":
                        raise ProtocolError("use active model; per-job model selection is unavailable")
                    if req["reference_id"] and req["reference_id"] not in self.voices:
                        raise ProtocolError("reference asset has no configured voice filename")
                    if req["voice_id"] and req["voice_id"] not in self.voices:
                        raise ProtocolError("voice id has no configured voice filename")
                job = Job(req)
                self.jobs[job_id] = job
                if not self.fixture:
                    self.pool.submit(self._run, job_id)
                return status(job_id, 1)
            job = self.jobs.get(job_id)
            if job is None:
                raise ProtocolError("unknown job")
            if req["kind"] == 2:
                job.state = 4  # Cancelled; any in-flight backend output is discarded.
                return status(job_id, 4)
            if self.fixture and job.state in (1, 2):
                job.fixture_polls += 1
                if job.fixture_polls == 1:
                    job.state, job.progress = 2, 5000
                else:
                    self._publish(job_id, job, FIXTURE_WAV, "fixture-v1")
            if job.state == 3:
                return result(job_id, job.asset_id, self.assets[job.asset_id], job.model, req_seed(job))
            return status(job_id, job.state, job.progress, job.error)

    def _publish(self, job_id: int, job: Job, wav: bytes, model: str):
        if job.state == 4:
            return
        asset_id = job_id  # Unique because duplicate job IDs are rejected.
        response = result(job_id, asset_id, wav, model, req_seed(job))
        frames = struct.unpack_from("<Q", response, 32)[0]
        if frames > job.request["max_frames"]:
            raise ProtocolError("generated audio exceeds requested frame bound")
        self.assets[asset_id] = wav
        job.asset_id, job.model, job.state, job.progress = asset_id, model, 3, 10000

    def _run(self, job_id: int):
        with self.lock:
            job = self.jobs[job_id]
            if job.state == 4:
                return
            job.state, job.progress = 2, 100
            req = job.request
        voice = self.voices.get(req["reference_id"] or req["voice_id"])
        payload = {"text": req["text"], "language": req["language"] or None,
                   "voice": voice, "mode": "clone" if voice else "direct"}
        request = urllib.request.Request(self.backend + "/api/synthesize",
                                         data=json.dumps(payload).encode(),
                                         headers={"Content-Type": "application/json"}, method="POST")
        try:
            # MOSSlanding returns 202 while model loading starts and 503 while
            # loading continues. Retry for at most two minutes, checking cancel.
            for attempt in range(60):
                with self.lock:
                    if job.state == 4:
                        return
                try:
                    with urllib.request.urlopen(request, timeout=300) as response:
                        if getattr(response, "status", 200) == 202:
                            wav = None
                        else:
                            wav = response.read(100_000_001)
                except urllib.error.HTTPError as exc:
                    if exc.code != 503:
                        raise
                    wav = None
                if wav is not None:
                    break
                if attempt == 59:
                    raise TimeoutError("MOSSlanding model loading timed out")
                time.sleep(2)
            if len(wav) > 100_000_000:
                raise ProtocolError("backend output too large")
            with urllib.request.urlopen(self.backend + "/api/status", timeout=5) as response:
                active_model = json.load(response).get("model_id", "active")
            with self.lock:
                self._publish(job_id, job, wav, active_model)
        except Exception:
            with self.lock:
                if job.state != 4:
                    job.state, job.error = 5, 1

    def asset(self, asset_id: int) -> bytes | None:
        with self.lock:
            return self.assets.get(asset_id)


def req_seed(job: Job) -> int:
    return job.request["seed"]


def serve(adapter: Adapter, host: str, port: int, token: str):
    if host not in ("127.0.0.1", "::1", "localhost") and not token:
        raise ValueError("non-loopback binding requires MOSS_VOICE_TOKEN")

    class Handler(BaseHTTPRequestHandler):
        def authorized(self) -> bool:
            if not token:
                return True
            given = self.headers.get("Authorization", "")
            return hmac.compare_digest(given, "Bearer " + token)

        def do_POST(self):
            if not self.authorized():
                self.send_error(401)
                return
            if self.path != "/v1/frame":
                self.send_error(404)
                return
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not 16 <= size <= MAX_FRAME:
                    raise ProtocolError("frame size")
                answer = adapter.handle(self.rfile.read(size))
            except (ValueError, ProtocolError) as exc:
                self.send_error(400, str(exc))
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(answer)))
            self.end_headers()
            self.wfile.write(answer)

        def do_GET(self):
            if not self.authorized():
                self.send_error(401)
                return
            if not self.path.startswith("/v1/assets/"):
                self.send_error(404)
                return
            try:
                asset_id = int(self.path.rsplit("/", 1)[1])
            except ValueError:
                self.send_error(400)
                return
            wav = adapter.asset(asset_id)
            if wav is None:
                self.send_error(404)
                return
            start, end = 0, len(wav) - 1
            range_header = self.headers.get("Range")
            if range_header is not None:
                try:
                    unit, value = range_header.split("=", 1)
                    first, last = value.split("-", 1)
                    if unit != "bytes" or not first or not last:
                        raise ValueError("invalid range")
                    start, end = int(first), int(last)
                    if start < 0 or end < start or start >= len(wav) or end - start >= 32768:
                        raise ValueError("range out of bounds")
                    end = min(end, len(wav) - 1)
                except ValueError:
                    self.send_error(416)
                    return
            part = wav[start:end + 1]
            self.send_response(206 if range_header is not None else 200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Content-Length", str(len(part)))
            self.send_header("Accept-Ranges", "bytes")
            if range_header is not None:
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(wav)}")
            self.end_headers()
            self.wfile.write(part)

    with ThreadingHTTPServer((host, port), Handler) as server:
        server.serve_forever()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", action="store_true")
    parser.add_argument("--backend", default="http://127.0.0.1:8765")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8875)
    args = parser.parse_args()
    voice_map = json.loads(os.environ.get("MOSS_VOICE_MAP", "{}"))
    voices = {}
    for key, filename in voice_map.items():
        if not isinstance(filename, str) or Path(filename).name != filename:
            raise ValueError("MOSS_VOICE_MAP values must be bare filenames")
        voices[int(key)] = filename
    adapter = Adapter(args.fixture, args.backend, voices)
    try:
        serve(adapter, args.host, args.port, os.environ.get("MOSS_VOICE_TOKEN", ""))
    finally:
        adapter.close()


if __name__ == "__main__":
    main()
