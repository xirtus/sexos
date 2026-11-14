#!/usr/bin/env python3
"""Host-side fake UVC Camera for Flash's gates and tests.

Implements the SexOS UVC fixture wire protocol over TCP:
- Emits real binary UVC descriptors (IAD, VideoControl, VideoStreaming YUY2 format & frames)
- Handles 26-byte / 34-byte Probe / Commit negotiation
- Streams multi-packet YUY2 video frames with valid UVC headers (FID toggle, EOF, PTS)
"""
import socket
import struct
import threading
import time

WIRE_MAGIC = b"SUVC"
MSG_DESCRIPTORS = 1
MSG_PROBE_REQ = 2
MSG_PROBE_RESP = 3
MSG_COMMIT_REQ = 4
MSG_COMMIT_ACK = 5
MSG_START_STREAM = 6
MSG_STOP_STREAM = 7
MSG_UVC_PACKET = 8

HEADER_INFO_FID = 0x01
HEADER_INFO_EOF = 0x02
HEADER_INFO_PTS = 0x04

GUID_YUY2 = bytes([
    0x59, 0x55, 0x59, 0x32, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71
])


def encode_wire_msg(msg_type, payload):
    return WIRE_MAGIC + bytes([msg_type, 0]) + struct.pack(">H", len(payload)) + payload


def build_uvc_descriptors(width=640, height=480, fps=30):
    d = bytearray()
    # 1. IAD (8 bytes)
    d.extend([0x08, 0x0B, 0x00, 0x02, 0x0E, 0x03, 0x00, 0x00])

    # 2. VC Standard Interface (9 bytes)
    d.extend([0x09, 0x04, 0x00, 0x00, 0x00, 0x0E, 0x01, 0x00, 0x00])

    # 3. VC Header (13 bytes)
    vc_total = 13 + 17 + 11 + 9
    d.extend([0x0D, 0x24, 0x01, 0x10, 0x01])
    d.extend(struct.pack("<H", vc_total))
    d.extend(struct.pack("<I", 48000000))
    d.extend([0x01, 0x01])

    # 4. Camera Input Terminal (17 bytes)
    d.extend([0x11, 0x24, 0x02, 0x01, 0x01, 0x02, 0x00, 0x00,
              0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00])

    # 5. Processing Unit (11 bytes)
    d.extend([0x0B, 0x24, 0x05, 0x02, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00])

    # 6. Output Terminal (9 bytes)
    d.extend([0x09, 0x24, 0x03, 0x03, 0x01, 0x01, 0x00, 0x02, 0x00])

    # 7. VS Standard Interface (9 bytes)
    d.extend([0x09, 0x04, 0x01, 0x00, 0x01, 0x0E, 0x02, 0x00, 0x00])

    # 8. VS Input Header (14 bytes)
    vs_total = 14 + 27 + 30
    d.extend([0x0E, 0x24, 0x01, 0x01])
    d.extend(struct.pack("<H", vs_total))
    d.extend([0x81, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00])

    # 9. VS Format Uncompressed YUY2 (27 bytes)
    d.extend([0x1B, 0x24, 0x04, 0x01, 0x01])
    d.extend(GUID_YUY2)
    d.extend([0x10, 0x01, 0x04, 0x03, 0x00, 0x00])

    # 10. VS Frame Uncompressed (30 bytes)
    interval_100ns = int(10000000 / fps)
    frame_size = width * height * 2
    d.extend([0x1E, 0x24, 0x05, 0x01, 0x00])
    d.extend(struct.pack("<H", width))
    d.extend(struct.pack("<H", height))
    d.extend(struct.pack("<I", frame_size * fps * 8))
    d.extend(struct.pack("<I", frame_size * fps * 8))
    d.extend(struct.pack("<I", frame_size))
    d.extend(struct.pack("<I", interval_100ns))
    d.extend([0x01])
    d.extend(struct.pack("<I", interval_100ns))

    # 11. Endpoint Descriptor (7 bytes)
    d.extend([0x07, 0x05, 0x81, 0x02, 0x00, 0x04, 0x00])

    return bytes(d)


def make_yuy2_frame(width=640, height=480, frame_num=0):
    out = bytearray(width * height * 2)
    shift = (frame_num * 8) % 256
    colors = [
        (128, 90, 240),   # Reddish
        (180, 54, 34),    # Greenish
        (80, 240, 110),   # Blueish
        (220, 128, 128),  # Bright White
    ]
    half_w = width // 2
    for y in range(height):
        row_y = y % 16
        for x in range(half_w):
            color_idx = (x * 8 // half_w + shift // 32) % 4
            y_val, u_val, v_val = colors[color_idx]
            idx = (y * width + x * 2) * 2
            out[idx] = (y_val + row_y) & 0xFF
            out[idx + 1] = u_val
            out[idx + 2] = (y_val + row_y) & 0xFF
            out[idx + 3] = v_val
    return bytes(out)


class FakeUvcCamera:
    def __init__(self, host="127.0.0.1", port=0, width=640, height=480, fps=30):
        self.host = host
        self.width = width
        self.height = height
        self.fps = fps
        self.server_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server_sock.bind((host, port))
        self.port = self.server_sock.getsockname()[1]
        self.server_sock.listen(5)
        self.running = True
        self.frames_sent = 0
        self.clients_served = 0
        self.thread = threading.Thread(target=self._accept_loop, daemon=True)
        self.thread.start()

    def _accept_loop(self):
        while self.running:
            try:
                conn, _ = self.server_sock.accept()
                self.clients_served += 1
                t = threading.Thread(target=self._client_handler, args=(conn,), daemon=True)
                t.start()
            except Exception:
                break

    def _client_handler(self, conn):
        try:
            # 1. Send Descriptors
            desc = build_uvc_descriptors(self.width, self.height, self.fps)
            conn.sendall(encode_wire_msg(MSG_DESCRIPTORS, desc))

            streaming = False
            rx_buf = bytearray()
            conn.settimeout(0.5)

            while self.running:
                try:
                    data = conn.recv(4096)
                    if not data:
                        break
                    rx_buf.extend(data)
                except socket.timeout:
                    pass

                # Parse wire messages
                while len(rx_buf) >= 8:
                    if rx_buf[:4] != WIRE_MAGIC:
                        rx_buf.clear()
                        break
                    msg_type = rx_buf[4]
                    plen = struct.unpack(">H", rx_buf[6:8])[0]
                    total_len = 8 + plen
                    if len(rx_buf) < total_len:
                        break
                    payload = bytes(rx_buf[8:total_len])
                    rx_buf = rx_buf[total_len:]

                    if msg_type == MSG_PROBE_REQ:
                        # Echo back probe response with requested / capped parameters
                        resp = bytearray(payload)
                        interval_100ns = int(10000000 / self.fps)
                        frame_size = self.width * self.height * 2
                        resp[4:8] = struct.pack("<I", interval_100ns)
                        resp[18:22] = struct.pack("<I", frame_size)
                        resp[22:26] = struct.pack("<I", 1024)
                        conn.sendall(encode_wire_msg(MSG_PROBE_RESP, bytes(resp)))

                    elif msg_type == MSG_COMMIT_REQ:
                        conn.sendall(encode_wire_msg(MSG_COMMIT_ACK, bytes([0])))

                    elif msg_type == MSG_START_STREAM:
                        streaming = True
                        break

                    elif msg_type == MSG_STOP_STREAM:
                        streaming = False
                        break

                if streaming:
                    break

            # Streaming loop
            frame_num = 0
            packet_size = 1024
            payload_per_pkt = packet_size - 6
            interval = 1.0 / self.fps

            while self.running and streaming:
                frame_data = make_yuy2_frame(self.width, self.height, frame_num)
                fid = (frame_num % 2) != 0
                pts = (frame_num * int(10000000 / self.fps)) & 0xFFFFFFFF

                offset = 0
                while offset < len(frame_data):
                    chunk_len = min(len(frame_data) - offset, payload_per_pkt)
                    is_eof = (offset + chunk_len >= len(frame_data))

                    info = HEADER_INFO_PTS
                    if fid:
                        info |= HEADER_INFO_FID
                    if is_eof:
                        info |= HEADER_INFO_EOF

                    header = bytes([6, info]) + struct.pack("<I", pts)
                    pkt = header + frame_data[offset:offset + chunk_len]
                    conn.sendall(encode_wire_msg(MSG_UVC_PACKET, pkt))
                    offset += chunk_len

                self.frames_sent += 1
                frame_num += 1
                time.sleep(interval)

        except Exception:
            pass
        finally:
            try:
                conn.close()
            except Exception:
                pass

    def stop(self):
        self.running = False
        try:
            self.server_sock.close()
        except Exception:
            pass
