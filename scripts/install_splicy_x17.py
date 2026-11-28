#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Install Splicy and a synthetic sample clip + transcript onto an X17 SexOS target.

Builds a local payload bundle:
  - /splicy        (`target/splicy/x86_64-sex-app/release/splicy`)
  - /splicy-studio (optional, if built at `apps/splicy-studio/target/x86_64-sex-std/release/splicy-studio`)
  - /talk.webm     (6 s synthetic VP9 + Opus clip generated with host `ffmpeg`)
  - /talk.words    (matching word-timestamp transcript)

Usage:
  # Serve over HTTP on the Mac's X17 LAN IP (192.168.1.x preferred) and print Spindle commands:
  python3 scripts/install_splicy_x17.py --serve --port 8787

  # Auto-pick an ephemeral port and print exact Spindle `fetch` / `run` commands:
  python3 scripts/install_splicy_x17.py

  # Push sample assets via active tether (`scripts/tether_host.py`) and serve/fetch large ELF:
  python3 scripts/install_splicy_x17.py --push --tether-dir tether-logs
"""

from __future__ import annotations

import argparse
import functools
import http.server
import ipaddress
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SPLICY = ROOT / "target/splicy/x86_64-sex-app/release/splicy"
FALLBACK_SPLICY = ROOT / "target/x86_64-sex-app/release/splicy"
DEFAULT_STUDIO = ROOT / "apps/splicy-studio/target/x86_64-sex-std/release/splicy-studio"
TETHER_HOST = ROOT / "scripts/tether_host.py"
TETHER_PUSHFILE_MAX_BYTES = 16 * 1024 * 1024

DEFAULT_WORDS = """# START_MS END_MS WORD
0 400 Hello
500 900 there
1000 1600 friends.
2000 2300 Cut
2400 2700 this
2800 3300 sentence
3400 3800 out.
4000 4400 Keep
4500 4800 the
4900 5400 ending
5500 5900 here.
"""


def candidate_ipv4_addresses() -> List[str]:
    """Return non-loopback, non-link-local IPv4 addresses on this host."""
    found: List[str] = []

    def add(ip: str) -> None:
        ip = ip.strip()
        try:
            addr = ipaddress.IPv4Address(ip)
        except ipaddress.AddressValueError:
            return
        if addr.is_loopback or addr.is_link_local or addr.is_unspecified:
            return
        if ip not in found:
            found.append(ip)

    # 1. Outward route probe (no packets sent).
    for target in ("192.168.1.1", "192.0.2.1"):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                s.connect((target, 9))
                add(s.getsockname()[0])
        except OSError:
            pass

    # 2. Hostname resolution.
    try:
        for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET):
            add(info[4][0])
    except OSError:
        pass

    # 3. Interface addresses on macOS / Linux (`ifconfig` / `ip addr`).
    for cmd in (["ifconfig"], ["ip", "-4", "addr"]):
        exe = shutil.which(cmd[0])
        if not exe:
            continue
        try:
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=3, check=False)
            for m in re.finditer(r"\binet\s+(\d+\.\d+\.\d+\.\d+)", res.stdout):
                add(m.group(1))
        except OSError:
            pass

    return found


def detect_lan_ip(preferred_prefix: str = "192.168.1.") -> str:
    """Pick the Mac's LAN IP, preferring the X17 subnet (192.168.1.x)."""
    candidates = candidate_ipv4_addresses()
    for ip in candidates:
        if ip.startswith(preferred_prefix):
            return ip
    for prefix in ("192.168.", "10.", "172."):
        for ip in candidates:
            if ip.startswith(prefix):
                return ip
    return candidates[0] if candidates else "127.0.0.1"


def resolve_splicy_binary(explicit: Optional[Path]) -> Path:
    """Resolve the Splicy ELF binary path or raise a helpful error."""
    if explicit is not None:
        if not explicit.is_file():
            raise FileNotFoundError(f"Splicy binary not found: {explicit}")
        return explicit

    env_path = os.environ.get("SEXOS_SPLICY")
    if env_path:
        p = Path(env_path)
        if not p.is_file():
            raise FileNotFoundError(f"SEXOS_SPLICY does not exist: {p}")
        return p

    for candidate in (DEFAULT_SPLICY, FALLBACK_SPLICY):
        if candidate.is_file():
            return candidate

    raise FileNotFoundError(
        f"Splicy binary not found at {DEFAULT_SPLICY}. Build it first with: scripts/build_splicy.sh"
    )


def resolve_studio_binary(explicit: Optional[Path], include_studio: bool) -> Optional[Path]:
    """Resolve optional splicy-studio ELF binary if requested/available."""
    if not include_studio:
        return None
    if explicit is not None:
        if not explicit.is_file():
            raise FileNotFoundError(f"splicy-studio binary not found: {explicit}")
        return explicit
    env_path = os.environ.get("SEXOS_SPLICY_STUDIO")
    if env_path:
        p = Path(env_path)
        if not p.is_file():
            raise FileNotFoundError(f"SEXOS_SPLICY_STUDIO does not exist: {p}")
        return p
    return DEFAULT_STUDIO if DEFAULT_STUDIO.is_file() else None


def generate_sample_clip(dest: Path) -> Path:
    """Generate a 6 s VP9 + Opus synthetic clip matching program_splicy_gate.py."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise FileNotFoundError("ffmpeg not found in PATH; pass --clip PATH or --no-sample")
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            ffmpeg,
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=6",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=6",
            "-af",
            "volume='7*(0.1+0.9*mod(t,1))':eval=frame",
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "300k",
            "-c:a",
            "libopus",
            "-ac",
            "2",
            str(dest),
        ],
        check=True,
    )
    return dest


def build_payloads(
    splicy_path: Path,
    studio_path: Optional[Path],
    clip_path: Optional[Path],
    words_path: Optional[Path],
    include_sample: bool,
    scratch_dir: Path,
) -> Dict[str, Tuple[Path, str]]:
    """Build URL route -> (local_path, sexos_dest_path) map."""
    payloads: Dict[str, Tuple[Path, str]] = {
        "/splicy": (splicy_path, "/bin/splicy"),
    }
    if studio_path is not None:
        payloads["/splicy-studio"] = (studio_path, "/bin/splicy-studio")

    if include_sample:
        if clip_path is not None:
            if not clip_path.is_file():
                raise FileNotFoundError(f"Sample clip not found: {clip_path}")
            clip_file = clip_path
        else:
            clip_file = generate_sample_clip(scratch_dir / "talk.webm")

        if words_path is not None:
            if not words_path.is_file():
                raise FileNotFoundError(f"Transcript file not found: {words_path}")
            words_file = words_path
        else:
            words_file = scratch_dir / "talk.words"
            words_file.write_text(DEFAULT_WORDS, encoding="utf-8")

        payloads["/talk.webm"] = (clip_file, "talk.webm")
        payloads["/talk.words"] = (words_file, "talk.words")

    return payloads


def spindle_commands(host: str, port: int, payloads: Dict[str, Tuple[Path, str]]) -> List[str]:
    """Return exact Spindle commands to fetch and launch Splicy on SexOS."""
    cmds = [f"fetch http://{host}:{port}{route} {dest}" for route, (_, dest) in payloads.items()]
    if "/talk.webm" in payloads and "/talk.words" in payloads:
        cmds.append("run splicy @talk.webm @talk.words")
    else:
        cmds.append("run splicy")
    return cmds


class SplicyHandler(http.server.BaseHTTPRequestHandler):
    """Serve Splicy binaries and sample clip/transcript over HTTP."""

    def __init__(self, *args, payloads: Dict[str, Tuple[Path, str]], stats: Dict[str, int], **kwargs):
        self.payloads = payloads
        self.stats = stats
        super().__init__(*args, **kwargs)

    def do_GET(self) -> None:  # noqa: N802
        route = self.path.split("?", 1)[0]
        if route == "/":
            listing = "\n".join(f"{r} -> {dest} ({p.stat().st_size} bytes)" for r, (p, dest) in self.payloads.items())
            body = (listing + "\n").encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        entry = self.payloads.get(route)
        if entry is None:
            self.send_response(404)
            self.end_headers()
            return

        path, _ = entry
        size = path.stat().st_size
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(size))
        self.end_headers()
        with path.open("rb") as f:
            shutil.copyfileobj(f, self.wfile, length=256 * 1024)
        self.stats[route] = self.stats.get(route, 0) + 1
        print(f"[http] served {route} ({size} bytes) to {self.client_address[0]}", flush=True)

    def log_message(self, fmt: str, *args) -> None:  # noqa: A002
        return


class ReusableThreadingHTTPServer(http.server.ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True


def start_http_server(
    bind_host: str, port: int, payloads: Dict[str, Tuple[Path, str]]
) -> Tuple[ReusableThreadingHTTPServer, Dict[str, int]]:
    stats: Dict[str, int] = {}
    handler = functools.partial(SplicyHandler, payloads=payloads, stats=stats)
    server = ReusableThreadingHTTPServer((bind_host, port), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, stats


def run_tether(tether_dir: Path, *args: str) -> int:
    cmd = [sys.executable, str(TETHER_HOST), "--dir", str(tether_dir), *args]
    proc = subprocess.run(cmd, check=False)
    return proc.returncode


def push_via_tether(
    tether_dir: Path,
    host: str,
    port: int,
    payloads: Dict[str, Tuple[Path, str]],
    send_fetch: bool,
) -> int:
    """Push files <= 16 MiB via tether_host.py; fetch larger ELFs over HTTP."""
    if not TETHER_HOST.is_file():
        print(f"error: tether host script missing: {TETHER_HOST}", file=sys.stderr)
        return 2

    need_http: Dict[str, Tuple[Path, str]] = {}
    for route, (path, dest) in payloads.items():
        size = path.stat().st_size
        if size <= TETHER_PUSHFILE_MAX_BYTES:
            print(f"[tether] pushing {path.name} ({size} bytes) -> {dest}", flush=True)
            rc = run_tether(tether_dir, "push-file", str(path), dest)
            if rc != 0:
                print(f"[tether] push-file failed for {route} (exit={rc}); falling back to HTTP fetch", file=sys.stderr)
                need_http[route] = (path, dest)
        else:
            print(
                f"[tether] {route} is {size} bytes (> {TETHER_PUSHFILE_MAX_BYTES} B push-file limit); "
                "using HTTP fetch",
                flush=True,
            )
            need_http[route] = (path, dest)

    if need_http:
        server, stats = start_http_server("0.0.0.0", port, need_http)
        actual_port = server.server_address[1]
        try:
            for route, (_, dest) in need_http.items():
                fetch_cmd = f"fetch http://{host}:{actual_port}{route} {dest}"
                print(f"Spindle command: {fetch_cmd}", flush=True)
                if send_fetch:
                    rc = run_tether(tether_dir, "send", fetch_cmd, "--wait", "60")
                    if rc != 0:
                        return rc
            if not send_fetch:
                print("Waiting for SexOS fetch requests (Ctrl+C to stop)...", flush=True)
                while not all(stats.get(r, 0) > 0 for r in need_http):
                    time.sleep(0.25)
        finally:
            server.shutdown()
            server.server_close()

    run_cmd = "run splicy @talk.webm @talk.words" if "/talk.webm" in payloads else "run splicy"
    print(f"Ready on SexOS! Run in Spindle: {run_cmd}", flush=True)
    return 0


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument("--splicy", type=Path, help=f"Path to splicy ELF (default: {DEFAULT_SPLICY})")
    p.add_argument("--studio", type=Path, help=f"Optional path to splicy-studio ELF (default: {DEFAULT_STUDIO})")
    p.add_argument("--no-studio", action="store_true", help="Do not include splicy-studio even if built")
    p.add_argument("--clip", type=Path, help="Existing sample video clip instead of generating talk.webm via ffmpeg")
    p.add_argument("--words", type=Path, help="Existing .words transcript instead of default talk.words")
    p.add_argument("--no-sample", action="store_true", help="Serve/push binaries only (skip talk.webm + talk.words)")
    p.add_argument("--host", help="Mac LAN IP to advertise in Spindle fetch URLs (auto-detected by default)")
    p.add_argument("--bind", default="0.0.0.0", help="HTTP bind address (default: 0.0.0.0)")
    p.add_argument("--port", type=int, default=0, help="HTTP port (default: 0 = ephemeral port)")
    p.add_argument(
        "--serve",
        action="store_true",
        help="Run HTTP server in the foreground for live X17 tether / sexos_exec fetching",
    )
    p.add_argument(
        "--duration",
        type=float,
        default=0.0,
        help="Optional max seconds to serve when --serve is set (0 = until Ctrl+C / SIGTERM)",
    )
    p.add_argument(
        "--push",
        action="store_true",
        help="Push assets <= 16 MiB via scripts/tether_host.py and serve/fetch larger binaries over HTTP",
    )
    p.add_argument(
        "--send-fetch",
        action="store_true",
        help="With --push, also dispatch `fetch ...` commands over tether_host.py send",
    )
    p.add_argument(
        "--tether-dir",
        type=Path,
        default=Path(os.environ.get("SEXOS_TETHER_DIR", "tether-logs")),
        help="Tether state/log directory (default: $SEXOS_TETHER_DIR or tether-logs)",
    )
    p.add_argument(
        "--print-commands",
        action="store_true",
        help="Print resolved files, detected IP, and Spindle commands without starting the server",
    )
    return p.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    host = args.host or detect_lan_ip()

    with tempfile.TemporaryDirectory(prefix="splicy-x17-") as tmp:
        scratch = Path(tmp)
        splicy_path = resolve_splicy_binary(args.splicy)
        studio_path = resolve_studio_binary(args.studio, include_studio=not args.no_studio)
        payloads = build_payloads(
            splicy_path=splicy_path,
            studio_path=studio_path,
            clip_path=args.clip,
            words_path=args.words,
            include_sample=not args.no_sample,
            scratch_dir=scratch,
        )

        print(f"Mac LAN IP: {host}", flush=True)
        for route, (path, dest) in payloads.items():
            print(f"  {route} <- {path} ({path.stat().st_size} bytes) -> {dest}", flush=True)

        if args.print_commands:
            port = args.port or 8787
            print("\nSpindle commands (X17):")
            for cmd in spindle_commands(host, port, payloads):
                print(f"  {cmd}")
            return 0

        if args.push:
            return push_via_tether(args.tether_dir, host, args.port, payloads, args.send_fetch)

        server, _ = start_http_server(args.bind, args.port, payloads)
        actual_port = server.server_address[1]
        cmds = spindle_commands(host, actual_port, payloads)
        print(f"\nServing Splicy X17 payload on http://{host}:{actual_port}/", flush=True)
        print("Spindle commands (run on X17 or via sexos_exec):", flush=True)
        for cmd in cmds:
            print(f"  {cmd}", flush=True)

        stop_event = threading.Event()

        def handle_sig(_signum, _frame) -> None:
            stop_event.set()

        signal.signal(signal.SIGINT, handle_sig)
        signal.signal(signal.SIGTERM, handle_sig)

        try:
            if args.serve or args.duration == 0.0:
                timeout = args.duration if args.duration > 0 else None
                stop_event.wait(timeout=timeout)
            else:
                stop_event.wait(timeout=args.duration)
        finally:
            server.shutdown()
            server.server_close()

    return 0


if __name__ == "__main__":
    sys.exit(main())
