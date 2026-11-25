#!/usr/bin/env python3
"""QEMU virtio-net throughput & latency benchmark gate for SexOS.

Boots a SexOS ISO in QEMU with modern virtio-net-pci (0x1af4:0x1041,
disable-legacy=on) — or optionally e1000e/e1000 for A/B comparison —
verifies kernel admission and link-up in serial logs, then drives real host
TCP traffic against sexnet's echo endpoint (10.0.2.15:46076):
  1. TCP round-trip latency (p50, p95, p99 in ms/us) over N ping-pong exchanges.
  2. Sustained full-duplex TCP bulk echo throughput (KiB/s and MiB/s).

Writes structured JSON results to --out/result.json and enforces optional
--min-throughput-kib and --max-p50-ms regression thresholds.
"""

import argparse
import hashlib
import json
import math
import pathlib
import re
import shutil
import socket
import subprocess
import threading
import time


def percentile(sorted_vals, pct):
    if not sorted_vals:
        return 0.0
    idx = (len(sorted_vals) - 1) * (pct / 100.0)
    lo = math.floor(idx)
    hi = math.ceil(idx)
    if lo == hi:
        return sorted_vals[lo]
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * (idx - lo)


def recv_exact(sock, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise RuntimeError(f"unexpected EOF after {len(buf)}/{n} bytes")
        buf.extend(chunk)
    return bytes(buf)


def measure_latency(port, iters, payload_bytes=64):
    rtts_us = []
    with socket.create_connection(("127.0.0.1", port), timeout=15) as sock:
        sock.settimeout(15)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        # Warm-up exchange so ARP/TCP handshake are excluded from steady-state RTT.
        warm = b"W" * payload_bytes
        sock.sendall(warm)
        if recv_exact(sock, payload_bytes) != warm:
            raise RuntimeError("latency warm-up echo mismatch")

        for i in range(iters):
            msg = bytes(((i + j * 13) & 0xFF) for j in range(payload_bytes))
            t0 = time.perf_counter_ns()
            sock.sendall(msg)
            reply = recv_exact(sock, payload_bytes)
            t1 = time.perf_counter_ns()
            if reply != msg:
                raise RuntimeError(f"latency echo mismatch at iter {i}")
            rtts_us.append((t1 - t0) / 1000.0)
        sock.shutdown(socket.SHUT_WR)
        # Drain FIN cleanly.
        while sock.recv(256):
            pass

    s = sorted(rtts_us)
    return {
        "iterations": iters,
        "payload_bytes": payload_bytes,
        "min_us": round(s[0], 2),
        "p50_us": round(percentile(s, 50), 2),
        "p95_us": round(percentile(s, 95), 2),
        "p99_us": round(percentile(s, 99), 2),
        "max_us": round(s[-1], 2),
        "p50_ms": round(percentile(s, 50) / 1000.0, 3),
        "p95_ms": round(percentile(s, 95) / 1000.0, 3),
        "p99_ms": round(percentile(s, 99) / 1000.0, 3),
    }


def measure_throughput(port, total_bytes, chunk_bytes=4096):
    payload = bytes(((i * 31 + 7) & 0xFF) for i in range(total_bytes))
    expected_sha = hashlib.sha256(payload).hexdigest()
    received = bytearray()
    recv_err = []

    with socket.create_connection(("127.0.0.1", port), timeout=15) as sock:
        sock.settimeout(30)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

        def reader():
            try:
                while len(received) < total_bytes:
                    part = sock.recv(min(chunk_bytes, total_bytes - len(received)))
                    if not part:
                        raise RuntimeError(f"bulk EOF at {len(received)}/{total_bytes}")
                    received.extend(part)
            except Exception as exc:
                recv_err.append(exc)

        t0 = time.perf_counter()
        t_rx = threading.Thread(target=reader, daemon=True)
        t_rx.start()
        try:
            view = memoryview(payload)
            sent = 0
            while sent < total_bytes:
                end = min(sent + chunk_bytes, total_bytes)
                sock.sendall(view[sent:end])
                sent = end
            sock.shutdown(socket.SHUT_WR)
        finally:
            t_rx.join(timeout=60)
        elapsed = time.perf_counter() - t0

    if recv_err:
        raise RuntimeError(f"bulk receiver failed: {recv_err[0]}")
    if len(received) != total_bytes:
        raise RuntimeError(f"bulk length mismatch: {len(received)} != {total_bytes}")
    actual_sha = hashlib.sha256(received).hexdigest()
    if actual_sha != expected_sha:
        raise RuntimeError("bulk payload SHA-256 mismatch")

    # Echo transfers total_bytes TX + total_bytes RX; report payload KiB/s (and round-trip KiB/s).
    kib = total_bytes / 1024.0
    kib_s = kib / elapsed if elapsed > 0 else 0.0
    return {
        "bytes": total_bytes,
        "elapsed_s": round(elapsed, 4),
        "throughput_kib_s": round(kib_s, 2),
        "throughput_mib_s": round(kib_s / 1024.0, 4),
        "bidirectional_kib_s": round(kib_s * 2.0, 2),
        "sha256": actual_sha,
    }


def main():
    p = argparse.ArgumentParser(description="SexOS QEMU virtio-net throughput & latency benchmark gate")
    p.add_argument("--iso", required=True, type=pathlib.Path)
    p.add_argument("--source-disk", type=pathlib.Path, default=None)
    p.add_argument("--out", required=True, type=pathlib.Path)
    p.add_argument("--nic", choices=["virtio-net-pci", "e1000e", "e1000"], default="virtio-net-pci")
    p.add_argument("--ping-iters", type=int, default=40)
    p.add_argument("--bulk-bytes", type=int, default=256 * 1024)
    p.add_argument("--boot-timeout", type=int, default=60)
    p.add_argument("--min-throughput-kib", type=float, default=0.0)
    p.add_argument("--max-p50-ms", type=float, default=0.0)
    args = p.parse_args()

    iso = args.iso.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    disk = out / "nvme.img"
    if args.source_disk is not None:
        shutil.copyfile(args.source_disk.resolve(strict=True), disk)
    else:
        with disk.open("wb") as f:
            f.truncate(64 * 1024 * 1024)

    serial = out / "serial.log"
    with socket.socket() as reserve:
        reserve.bind(("127.0.0.1", 0))
        port = reserve.getsockname()[1]

    nic_dev = (
        f"{args.nic},netdev=n0,disable-legacy=on,mac=52:54:00:12:34:56"
        if args.nic == "virtio-net-pci"
        else f"{args.nic},netdev=n0,mac=52:54:00:12:34:56"
    )
    cmd = [
        "qemu-system-x86_64",
        "-M", "q35",
        "-cpu", "max,+pku",
        "-m", "512M",
        "-cdrom", str(iso),
        "-device", "nec-usb-xhci,id=xhci",
        "-device", "usb-tablet,bus=xhci.0",
        "-drive", f"if=none,id=nvm,file={disk},format=raw",
        "-device", "nvme,serial=sexos01,drive=nvm",
        "-serial", f"file:{serial}",
        "-display", "none",
        "-no-reboot",
        "-no-shutdown",
        "-nic", "none",
        "-netdev", f"user,id=n0,hostfwd=tcp:127.0.0.1:{port}-10.0.2.15:46076",
        "-device", nic_dev,
    ]

    qemu_ver = subprocess.check_output(["qemu-system-x86_64", "--version"], text=True).splitlines()[0]
    (out / "manifest.json").write_text(json.dumps({
        "nic": args.nic,
        "iso_sha256": hashlib.sha256(iso.read_bytes()).hexdigest(),
        "command": cmd,
        "qemu_version": qemu_ver,
    }, indent=2) + "\n")

    proc = subprocess.Popen(cmd, stdout=(out / "qemu.log").open("w"), stderr=subprocess.STDOUT)

    def read_log():
        return serial.read_text(errors="replace") if serial.exists() else ""

    def wait_for(pattern, timeout=args.boot_timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                raise RuntimeError("QEMU guest exited unexpectedly")
            text = read_log()
            if re.search(r"KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|to=Quarantined|sexnet\.transport\.failed", text):
                raise RuntimeError("guest/device/protocol failure in serial log")
            m = re.search(pattern, text)
            if m:
                return m
            time.sleep(0.02)
        raise TimeoutError(f"timed out waiting for pattern: {pattern}")

    boot_t0 = time.monotonic()
    try:
        if args.nic == "virtio-net-pci":
            wait_for(r"\[kernel\.cap\.virtio\.grant\] pd=13 slot=19 product=0x1041\b")
            wait_for(r"\[sexnet\.virtio\.ready\] vendor=1af4 product=1041\b")
            wait_for(r"\[sexnet\.ether\.ready\] model=Virtio\b")
        else:
            wait_for(r"\[kernel\.cap\.nic\.grant\] pd=13 slot=19\b")
            wait_for(r"\[sexnet\.ether\.ready\]")
        wait_for(r"\[sexnet\.transport\.ready\] .*port=46076\b")
        boot_ready_s = round(time.monotonic() - boot_t0, 3)

        # Wait briefly for sexnet's initial ARP/IRQ setup to settle.
        time.sleep(0.25)

        latency = measure_latency(port, args.ping_iters)
        # Wait for the diagnostic listener to return to Listen before opening the bulk flow.
        wait_for(r"\[sexnet\.transport\.state\] from=\w+ to=Listen flow=1\b", timeout=15)
        throughput = measure_throughput(port, args.bulk_bytes)
        wait_for(r"\[sexnet\.transport\.state\] from=\w+ to=Listen flow=2\b", timeout=15)

        log_text = read_log()
        irq_armed = bool(re.search(r"\[sexnet\.irq\] armed=1\b", log_text))
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()

    errors = []
    if args.min_throughput_kib > 0 and throughput["throughput_kib_s"] < args.min_throughput_kib:
        errors.append(
            f"throughput {throughput['throughput_kib_s']} KiB/s < threshold {args.min_throughput_kib} KiB/s"
        )
    if args.max_p50_ms > 0 and latency["p50_ms"] > args.max_p50_ms:
        errors.append(
            f"p50 latency {latency['p50_ms']} ms > threshold {args.max_p50_ms} ms"
        )

    result = {
        "ok": not errors,
        "errors": errors,
        "nic": args.nic,
        "qemu_version": qemu_ver,
        "boot_ready_s": boot_ready_s,
        "irq_armed": irq_armed,
        "latency": latency,
        "throughput": throughput,
        "thresholds": {
            "min_throughput_kib": args.min_throughput_kib,
            "max_p50_ms": args.max_p50_ms,
        },
    }
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")

    print("\n=== SexOS QEMU Network Benchmark Summary ===")
    print(f"NIC Model       : {args.nic} (IRQ armed: {irq_armed})")
    print(f"Boot to TCP/IP  : {boot_ready_s:.3f} s")
    print(
        f"TCP RTT ({latency['iterations']}x {latency['payload_bytes']}B): "
        f"p50={latency['p50_ms']:.3f} ms ({latency['p50_us']:.1f} us)  "
        f"p95={latency['p95_ms']:.3f} ms  p99={latency['p99_ms']:.3f} ms  "
        f"min={latency['min_us']:.1f} us  max={latency['max_us']:.1f} us"
    )
    print(
        f"TCP Bulk ({throughput['bytes'] // 1024} KiB): "
        f"{throughput['throughput_kib_s']:.2f} KiB/s ({throughput['throughput_mib_s']:.3f} MiB/s) "
        f"in {throughput['elapsed_s']:.3f} s  [bidir={throughput['bidirectional_kib_s']:.2f} KiB/s]"
    )
    print(f"Status          : {'PASS' if result['ok'] else 'FAIL: ' + '; '.join(errors)}")
    print(f"Results JSON    : {out / 'result.json'}\n")

    if not result["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
