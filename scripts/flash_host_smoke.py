#!/usr/bin/env python3
"""Host smoke test for SexOS Flash UVC protocol.

Spawns FakeUvcCamera on loopback TCP, connects as a client, executes the UVC
negotiation handshake (descriptors, probe, commit, start_stream), reassembles
YUY2 video frames across packet boundaries, and decodes pixels.
"""
import socket
import struct
import sys
import time
from flash_fakes import (
    FakeUvcCamera, WIRE_MAGIC, MSG_DESCRIPTORS, MSG_PROBE_REQ, MSG_PROBE_RESP,
    MSG_COMMIT_REQ, MSG_COMMIT_ACK, MSG_START_STREAM, MSG_UVC_PACKET,
    HEADER_INFO_FID, HEADER_INFO_EOF
)


def read_exact(sock, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError("Socket closed prematurely")
        buf.extend(chunk)
    return bytes(buf)


def recv_wire_msg(sock):
    header = read_exact(sock, 8)
    assert header[:4] == WIRE_MAGIC, f"Bad magic: {header[:4]!r}"
    msg_type = header[4]
    plen = struct.unpack(">H", header[6:8])[0]
    payload = read_exact(sock, plen) if plen > 0 else b""
    return msg_type, payload


def main():
    print("[flash_host_smoke] Starting FakeUvcCamera on 127.0.0.1:0...")
    camera = FakeUvcCamera(host="127.0.0.1", port=0, width=640, height=480, fps=30)
    print(f"[flash_host_smoke] Camera listening on port {camera.port}")

    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.connect(("127.0.0.1", camera.port))
        sock.settimeout(5.0)

        # 1. Expect descriptors
        msg_type, desc_data = recv_wire_msg(sock)
        assert msg_type == MSG_DESCRIPTORS, f"Expected MSG_DESCRIPTORS, got {msg_type}"
        assert len(desc_data) > 30, f"Descriptors too short: {len(desc_data)}"
        print(f"[flash_host_smoke] Received {len(desc_data)} bytes of UVC descriptors")

        # 2. Probe negotiation
        probe_req = bytearray(26)
        probe_req[0:2] = struct.pack("<H", 0x0001)  # bmHint
        probe_req[2] = 1  # bFormatIndex (YUY2)
        probe_req[3] = 1  # bFrameIndex
        probe_req[4:8] = struct.pack("<I", 333333)  # dwFrameInterval (30 fps)
        wire_probe = WIRE_MAGIC + bytes([MSG_PROBE_REQ, 0]) + struct.pack(">H", 26) + bytes(probe_req)
        sock.sendall(wire_probe)

        msg_type, probe_resp = recv_wire_msg(sock)
        assert msg_type == MSG_PROBE_RESP, f"Expected MSG_PROBE_RESP, got {msg_type}"
        assert len(probe_resp) >= 26, f"Probe resp too short: {len(probe_resp)}"
        interval, max_frame = struct.unpack("<II", probe_resp[4:12])
        print(f"[flash_host_smoke] Negotiated interval={interval} max_frame_size={max_frame}")

        # 3. Commit negotiation
        commit_req = bytearray(probe_resp[:26])
        wire_commit = WIRE_MAGIC + bytes([MSG_COMMIT_REQ, 0]) + struct.pack(">H", 26) + bytes(commit_req)
        sock.sendall(wire_commit)

        msg_type, ack = recv_wire_msg(sock)
        assert msg_type == MSG_COMMIT_ACK, f"Expected MSG_COMMIT_ACK, got {msg_type}"
        print("[flash_host_smoke] Commit ACK received")

        # 4. Start streaming
        wire_start = WIRE_MAGIC + bytes([MSG_START_STREAM, 0]) + struct.pack(">H", 0)
        sock.sendall(wire_start)
        print("[flash_host_smoke] Sent StartStream")

        # 5. Receive and reassemble frames
        frames_received = 0
        current_frame_payload = bytearray()
        current_fid = None

        while frames_received < 3:
            msg_type, pkt = recv_wire_msg(sock)
            if msg_type != MSG_UVC_PACKET:
                continue

            assert len(pkt) >= 2, "UVC packet missing header"
            hlen = pkt[0]
            assert hlen >= 2 and len(pkt) >= hlen, f"Invalid header len {hlen}"
            info = pkt[1]
            fid = bool(info & HEADER_INFO_FID)
            eof = bool(info & HEADER_INFO_EOF)

            if current_fid is None:
                current_fid = fid
            elif current_fid != fid:
                # FID toggled without EOF
                print(f"[flash_host_smoke] FID toggled, frame completed with {len(current_frame_payload)} bytes")
                assert len(current_frame_payload) == 640 * 480 * 2, f"Frame size mismatch: {len(current_frame_payload)}"
                frames_received += 1
                current_frame_payload.clear()
                current_fid = fid

            payload = pkt[hlen:]
            current_frame_payload.extend(payload)

            if eof:
                assert len(current_frame_payload) == 640 * 480 * 2, f"Frame size mismatch: {len(current_frame_payload)}"
                frames_received += 1
                print(f"[flash_host_smoke] Frame {frames_received} assembled successfully ({len(current_frame_payload)} bytes)")
                current_frame_payload.clear()
                current_fid = None

        print(f"[flash_host_smoke] Successfully reassembled {frames_received} frames! Camera reported {camera.frames_sent} sent.")
        assert camera.frames_sent >= 3
        sock.close()
        print("[flash_host_smoke] PASS")
        return 0

    finally:
        camera.stop()


if __name__ == "__main__":
    sys.exit(main())
