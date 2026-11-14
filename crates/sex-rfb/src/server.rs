// SPDX-License-Identifier: GPL-3.0-only
//! The server's side of RFB: the messages a server sends (handshake, ServerInit,
//! FramebufferUpdate in Raw and ZRLE) and a parser for what clients send. Lurk's
//! server half will use these once it has screen and input grants.
use crate::{zrle, Error, PixelFormat, Reader};
use alloc::vec::Vec;
use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};

pub const VERSION: &[u8; 12] = b"RFB 003.008\n";

/// The security types offered (3.7 and later).
pub fn security_types(types: &[u8]) -> Vec<u8> {
    let mut out = alloc::vec![types.len() as u8];
    out.extend_from_slice(types);
    out
}

/// SecurityResult: 0 for success, otherwise 1 and the reason (3.8).
pub fn security_result(reason: Option<&[u8]>) -> Vec<u8> {
    match reason {
        None => 0u32.to_be_bytes().to_vec(),
        Some(why) => {
            let mut out = 1u32.to_be_bytes().to_vec();
            out.extend_from_slice(&(why.len() as u32).to_be_bytes());
            out.extend_from_slice(why);
            out
        }
    }
}

/// ServerInit: the screen's size, its pixel format and the desktop's name.
pub fn server_init(width: u16, height: u16, format: &PixelFormat, name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + name.len());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&format.to_bytes());
    out.extend_from_slice(&(name.len() as u32).to_be_bytes());
    out.extend_from_slice(name);
    out
}

/// The start of a FramebufferUpdate with `rects` rectangles.
pub fn update_header(rects: u16) -> [u8; 4] {
    let [a, b] = rects.to_be_bytes();
    [0, 0, a, b]
}

/// A rectangle's header.
pub fn rect_header(x: u16, y: u16, w: u16, h: u16, encoding: i32) -> [u8; 12] {
    let mut out = [0u8; 12];
    for (i, v) in [x, y, w, h].iter().enumerate() { out[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes()); }
    out[8..].copy_from_slice(&encoding.to_be_bytes());
    out
}

/// Raw pixels (RGB32) of the `w` by `h` block of `src` at `(x, y)`; rows `stride` apart.
pub fn encode_raw(src: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize, out: &mut Vec<u8>) {
    for row in y..y + h {
        for &p in &src[row * stride + x..row * stride + x + w] { out.extend_from_slice(&(p & 0x00FF_FFFF).to_le_bytes()); }
    }
}

/// ZRLE's tile data before zlib, for tests and for servers that compress elsewhere.
pub fn zrle_tiles(src: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize, out: &mut Vec<u8>) {
    zrle::encode(src, stride, x, y, w, h, out)
}

/// One connection's ZRLE encoder: one zlib stream for the whole connection, flushed
/// at the end of each rectangle as the protocol requires.
pub struct ZrleEncoder { zlib: alloc::boxed::Box<CompressorOxide>, tiles: Vec<u8> }

impl Default for ZrleEncoder { fn default() -> Self { Self::new() } }

impl ZrleEncoder {
    pub fn new() -> Self {
        // Level 6, with the zlib header (window bits > 0).
        let flags = create_comp_flags_from_zip_params(6, 15, 0);
        ZrleEncoder { zlib: alloc::boxed::Box::new(CompressorOxide::new(flags)), tiles: Vec::new() }
    }
    /// The rectangle's ZRLE data (length and zlib bytes), after its header.
    pub fn encode(&mut self, src: &[u32], stride: usize, x: usize, y: usize, w: usize, h: usize, out: &mut Vec<u8>) {
        self.tiles.clear();
        zrle::encode(src, stride, x, y, w, h, &mut self.tiles);
        let at = out.len();
        out.extend_from_slice(&[0; 4]);
        let mut input = &self.tiles[..];
        let mut chunk = [0u8; 4096];
        loop {
            let (status, used, made) = compress(&mut self.zlib, input, &mut chunk, TDEFLFlush::Sync);
            input = &input[used..];
            out.extend_from_slice(&chunk[..made]);
            if status != TDEFLStatus::Okay || (input.is_empty() && made < chunk.len()) { break; }
        }
        let n = (out.len() - at - 4) as u32;
        out[at..at + 4].copy_from_slice(&n.to_be_bytes());
    }
}

/// What a client sends after the handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<i32>),
    UpdateRequest { incremental: bool, x: u16, y: u16, w: u16, h: u16 },
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    CutText(Vec<u8>),
}

/// Parses one client message from the start of `buf`: the message and the bytes it
/// took, `Ok(None)` when it hasn't all arrived.
pub fn parse_client_message(buf: &[u8]) -> Result<Option<(ClientMessage, usize)>, Error> {
    let mut r = Reader::new(buf);
    let m = match parse(&mut r) {
        None => return Ok(None),
        Some(m) => m?,
    };
    Ok(Some((m, r.at)))
}

fn parse(r: &mut Reader) -> Option<Result<ClientMessage, Error>> {
    Some(Ok(match r.u8()? {
        0 => {
            r.bytes(3)?;
            ClientMessage::SetPixelFormat(PixelFormat::from_bytes(r.bytes(16)?.try_into().unwrap()))
        }
        2 => {
            r.u8()?;
            let n = r.u16()? as usize;
            let list = r.bytes(4 * n)?;
            ClientMessage::SetEncodings(list.chunks_exact(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())).collect())
        }
        3 => ClientMessage::UpdateRequest { incremental: r.u8()? != 0, x: r.u16()?, y: r.u16()?, w: r.u16()?, h: r.u16()? },
        4 => {
            let down = r.u8()? != 0;
            r.bytes(2)?;
            ClientMessage::Key { down, keysym: r.u32()? }
        }
        5 => ClientMessage::Pointer { buttons: r.u8()?, x: r.u16()?, y: r.u16()? },
        6 => {
            r.bytes(3)?;
            let n = r.u32()? as usize;
            if n > 1 << 24 { return Some(Err(Error::Protocol("cut text length"))); }
            ClientMessage::CutText(r.bytes(n)?.to_vec())
        }
        _ => return Some(Err(Error::Protocol("client message type"))),
    }))
}
