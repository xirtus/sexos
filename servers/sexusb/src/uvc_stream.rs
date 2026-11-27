// SPDX-License-Identifier: GPL-3.0-only
//! Pure UVC payload header stripping, frame reassembly, and VS_PROBE/VS_COMMIT wire helpers.
//!
//! No MMIO and no heap allocation so `tests/sexusb_uvc.rs` runs this exact file on the host.

pub const HDR_FID: u8 = 1 << 0;
pub const HDR_EOF: u8 = 1 << 1;
pub const HDR_PTS: u8 = 1 << 2;
pub const HDR_SCR: u8 = 1 << 3;
pub const HDR_STI: u8 = 1 << 5;
pub const HDR_ERR: u8 = 1 << 6;

pub const FLAG_SOF: u16 = 1 << 0;
pub const FLAG_EOF: u16 = 1 << 1;
pub const FLAG_ERR: u16 = 1 << 2;
pub const FLAG_FID: u16 = 1 << 3;

/// Maximum payload bytes per chunk pushed into the `sexusb <-> sexuvc` shared ring.
pub const MAX_CHUNK: usize = 4088;

/// UVC class-specific VideoStreaming control request constants.
pub const BM_REQ_SET_IFACE: u8 = 0x21;
pub const BM_REQ_GET_IFACE: u8 = 0xA1;
pub const REQ_SET_CUR: u8 = 0x01;
pub const REQ_GET_CUR: u8 = 0x81;
pub const VS_PROBE_CONTROL: u16 = 0x0100;
pub const VS_COMMIT_CONTROL: u16 = 0x0200;

/// Parsed 2..12 byte UVC payload header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UvcHeader {
    pub len: u8,
    pub info: u8,
    pub fid: bool,
    pub eof: bool,
    pub pts: Option<u32>,
    pub scr_stc: Option<u32>,
    pub scr_sof: Option<u16>,
    pub still: bool,
    pub err: bool,
}

/// Parse a UVC payload header (2–12+ bytes) and return `(header, stripped_payload)`.
pub fn parse_header(packet: &[u8]) -> Option<(UvcHeader, &[u8])> {
    if packet.len() < 2 {
        return None;
    }
    let hlen = packet[0] as usize;
    if hlen < 2 || hlen > packet.len() {
        return None;
    }
    let info = packet[1];
    let has_pts = (info & HDR_PTS) != 0;
    let has_scr = (info & HDR_SCR) != 0;
    let mut off = 2usize;
    let pts = if has_pts {
        if off + 4 > hlen {
            return None;
        }
        let v = u32::from_le_bytes([packet[off], packet[off + 1], packet[off + 2], packet[off + 3]]);
        off += 4;
        Some(v)
    } else {
        None
    };
    let (scr_stc, scr_sof) = if has_scr {
        if off + 6 > hlen {
            return None;
        }
        let stc = u32::from_le_bytes([packet[off], packet[off + 1], packet[off + 2], packet[off + 3]]);
        let sof = u16::from_le_bytes([packet[off + 4], packet[off + 5]]);
        (Some(stc), Some(sof))
    } else {
        (None, None)
    };
    let hdr = UvcHeader {
        len: hlen as u8,
        info,
        fid: (info & HDR_FID) != 0,
        eof: (info & HDR_EOF) != 0,
        pts,
        scr_stc,
        scr_sof,
        still: (info & HDR_STI) != 0,
        err: (info & HDR_ERR) != 0,
    };
    Some((hdr, &packet[hlen..]))
}

/// Build a 26-byte UVC 1.0 `VS_PROBE_CONTROL` / `VS_COMMIT_CONTROL` payload.
pub fn build_probe_commit(format_index: u8, frame_index: u8, fps: u16) -> [u8; 26] {
    let mut b = [0u8; 26];
    let interval = if fps == 0 { 333_333u32 } else { 10_000_000u32 / (fps as u32) };
    b[0..2].copy_from_slice(&1u16.to_le_bytes()); // bmHint: dwFrameInterval
    b[2] = format_index.max(1);
    b[3] = frame_index.max(1);
    b[4..8].copy_from_slice(&interval.to_le_bytes());
    b
}

/// Build a zero-extended 48-byte `VS_PROBE_CONTROL` / `VS_COMMIT_CONTROL` buffer.
/// Slice to `26` (UVC 1.0), `34` (UVC 1.1), or `48` (UVC 1.5) bytes according to `bcdUVC`.
pub fn build_probe_commit_48(format_index: u8, frame_index: u8, fps: u16) -> [u8; 48] {
    let mut b = [0u8; 48];
    b[..26].copy_from_slice(&build_probe_commit(format_index, frame_index, fps));
    b
}

/// Extract `(dwMaxVideoFrameSize, dwMaxPayloadTransferSize)` from a `VS_PROBE` GET_CUR reply.
pub fn parse_probe_response(buf: &[u8]) -> Option<(u32, u32)> {
    if buf.len() < 26 {
        return None;
    }
    let max_frame = u32::from_le_bytes([buf[18], buf[19], buf[20], buf[21]]);
    let max_payload = u32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]);
    Some((max_frame, max_payload))
}

/// Zero-allocation UVC isochronous/bulk payload stripper and chunked frame emitter.
pub struct UvcReassembler {
    buf: [u8; MAX_CHUNK],
    buf_len: usize,
    current_fid: Option<bool>,
    frame_seq: u32,
    emitted_any: bool,
    corrupted: bool,
    pub packets_seen: u64,
    pub frames_completed: u64,
    pub frames_dropped: u64,
}

impl UvcReassembler {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; MAX_CHUNK],
            buf_len: 0,
            current_fid: None,
            frame_seq: 1,
            emitted_any: false,
            corrupted: false,
            packets_seen: 0,
            frames_completed: 0,
            frames_dropped: 0,
        }
    }

    pub fn reset(&mut self) {
        self.buf_len = 0;
        self.current_fid = None;
        self.emitted_any = false;
        self.corrupted = false;
    }

    fn flush_chunk<F: FnMut(u16, u32, &[u8]) -> bool>(&mut self, eof: bool, fid: bool, emit: &mut F) {
        let mut flags = if fid { FLAG_FID } else { 0 };
        if !self.emitted_any {
            flags |= FLAG_SOF;
        }
        if eof {
            flags |= FLAG_EOF;
        }
        let ok = emit(flags, self.frame_seq, &self.buf[..self.buf_len]);
        self.buf_len = 0;
        self.emitted_any = true;
        if !ok {
            self.corrupted = true;
            self.frames_dropped += 1;
        }
    }

    /// Feed one raw USB isochronous/bulk IN packet (including its 2..12 byte UVC payload header).
    /// Calls `emit(flags, frame_seq, stripped_payload_chunk)` whenever a chunk or frame boundary is ready.
    pub fn feed<F: FnMut(u16, u32, &[u8]) -> bool>(&mut self, packet: &[u8], mut emit: F) {
        self.packets_seen += 1;
        let Some((hdr, mut payload)) = parse_header(packet) else {
            return;
        };

        // FID toggle signals a new frame; finalize previous frame if EOF was missed.
        if let Some(prev_fid) = self.current_fid {
            if prev_fid != hdr.fid {
                if !self.corrupted && (self.buf_len > 0 || self.emitted_any) {
                    self.flush_chunk(true, prev_fid, &mut emit);
                    if !self.corrupted {
                        self.frames_completed += 1;
                    }
                    self.frame_seq = self.frame_seq.wrapping_add(1).max(1);
                }
                self.buf_len = 0;
                self.emitted_any = false;
                self.corrupted = false;
                self.current_fid = Some(hdr.fid);
            }
        } else {
            self.current_fid = Some(hdr.fid);
            self.corrupted = false;
            self.emitted_any = false;
        }

        // Error bit drops the current frame cleanly.
        if hdr.err {
            if !self.corrupted {
                self.frames_dropped += 1;
                if self.emitted_any {
                    let _ = emit(FLAG_ERR | FLAG_EOF, self.frame_seq, &[]);
                    self.frame_seq = self.frame_seq.wrapping_add(1).max(1);
                }
            }
            self.corrupted = true;
            self.buf_len = 0;
            self.emitted_any = false;
            if hdr.eof {
                self.current_fid = None;
                self.corrupted = false;
            }
            return;
        }

        if self.corrupted {
            if hdr.eof {
                self.current_fid = None;
                self.corrupted = false;
            }
            return;
        }

        while !payload.is_empty() {
            let space = MAX_CHUNK - self.buf_len;
            let take = payload.len().min(space);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&payload[..take]);
            self.buf_len += take;
            payload = &payload[take..];
            if self.buf_len == MAX_CHUNK && (!payload.is_empty() || !hdr.eof) {
                self.flush_chunk(false, hdr.fid, &mut emit);
                if self.corrupted {
                    return;
                }
            }
        }

        if hdr.eof {
            if self.buf_len > 0 || self.emitted_any {
                self.flush_chunk(true, hdr.fid, &mut emit);
                if !self.corrupted {
                    self.frames_completed += 1;
                }
                self.frame_seq = self.frame_seq.wrapping_add(1).max(1);
            }
            self.buf_len = 0;
            self.emitted_any = false;
            self.current_fid = None;
            self.corrupted = false;
        }
    }
}
