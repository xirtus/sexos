// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A program's accessibility tree, to Shell's screen reader (Whisper).
//!
//! A program built on Weft describes its window as AccessKit nodes (`weft::a11y`).
//! Shell reads only what it needs out loud: the node that has keyboard focus, its name
//! and its role. The tree is the person's screen, so a program sends it only while the
//! person has a screen reader on, and only to Shell:
//!
//! 1. The program asks Shell once, when its window opens: [`ASK`] ("is a reader on?").
//! 2. Shell answers [`STATE`] with `arg1` = 1 (on) or 0 (off), and again each time the
//!    person turns the reader on or off. A program that never asked hears nothing, and
//!    a program that heard "off" sends nothing.
//! 3. While on, the program sends its whole tree once, then each update (what changed,
//!    always with the focus), as [`TREE`] pieces of the bytes `weft::a11y::encode`
//!    makes. Shell takes pieces only from a program that asked, while the reader is on.
//!
//! All messages are `OP_A11Y` (0x2DF) with the kind in `arg0` bits 56..64. A `TREE`
//! piece carries up to [`PIECE_BYTES`] bytes in `arg1`/`arg2` (little-endian); `arg0`
//! bits 0..8 count them, bit 8 [`FIRST`] starts an update and bit 9 [`LAST`] ends it.
//! An update past [`MAX_UPDATE`] bytes is dropped whole.
//!
//! The update bytes (`weft::a11y::encode`, version 1): "WA11", version, flags (bit 0: a
//! full tree), the root id if full, the focus id, a node count, then per node: id u64,
//! role u8 (AccessKit's `Role` as u8), flags u32 (bit 10: bounds follow as 4 f32, bit
//! 11: four f64 numbers follow the strings), label, value, description, placeholder and
//! shortcut (each u32 length + UTF-8), position, set size and level (u32 each), actions
//! u32, a child count u32 and the child ids. All little-endian. [`scan`] reads it.

pub const OP_A11Y: u64 = 0x2DF;
/// Program -> Shell: is a screen reader on? Shell answers with [`STATE`].
pub const ASK: u64 = 1;
/// Shell -> program: `arg1` = 1 a reader is on (send the tree), 0 none is (send nothing).
pub const STATE: u64 = 2;
/// Program -> Shell: a piece of an encoded tree update.
pub const TREE: u64 = 3;
pub const PIECE_BYTES: usize = 16;
pub const FIRST: u64 = 1 << 8;
pub const LAST: u64 = 1 << 9;
pub const MAX_UPDATE: usize = 64 * 1024;
pub const MAGIC: &[u8; 4] = b"WA11";
pub const VERSION: u8 = 1;

pub const fn kind(arg0: u64) -> u64 { arg0 >> 56 }
pub const fn ask() -> [u64; 3] { [ASK << 56, 0, 0] }
pub const fn state(on: bool) -> [u64; 3] { [STATE << 56, on as u64, 0] }
/// Shell's answer: Some(on) for a `STATE` message.
pub const fn decode_state(op: u64, arg0: u64, arg1: u64) -> Option<bool> {
    if op == OP_A11Y && kind(arg0) == STATE { Some(arg1 == 1) } else { None }
}

/// The `(arg0, arg1, arg2)` of each `TREE` message for one encoded update.
pub fn pieces(bytes: &[u8]) -> impl Iterator<Item = [u64; 3]> + '_ {
    let count = bytes.len().div_ceil(PIECE_BYTES).max(1);
    (0..count).map(move |i| {
        let chunk = &bytes[(i * PIECE_BYTES).min(bytes.len())..((i + 1) * PIECE_BYTES).min(bytes.len())];
        let mut b = [0u8; PIECE_BYTES];
        b[..chunk.len()].copy_from_slice(chunk);
        let mut head = TREE << 56 | chunk.len() as u64;
        if i == 0 { head |= FIRST; }
        if i + 1 == count { head |= LAST; }
        [head, u64::from_le_bytes(b[..8].try_into().unwrap()), u64::from_le_bytes(b[8..].try_into().unwrap())]
    })
}

/// One `TREE` piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece { pub first: bool, pub last: bool, pub len: usize, pub bytes: [u8; PIECE_BYTES] }
impl Piece {
    pub fn decode(arg0: u64, arg1: u64, arg2: u64) -> Option<Piece> {
        if kind(arg0) != TREE { return None; }
        let len = (arg0 & 0xFF) as usize;
        if len > PIECE_BYTES { return None; }
        let mut bytes = [0u8; PIECE_BYTES];
        bytes[..8].copy_from_slice(&arg1.to_le_bytes());
        bytes[8..].copy_from_slice(&arg2.to_le_bytes());
        Some(Piece { first: arg0 & FIRST != 0, last: arg0 & LAST != 0, len, bytes })
    }
    pub fn data(&self) -> &[u8] { &self.bytes[..self.len] }
}

/// What [`scan`] found in an update besides its nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head { pub full: bool, pub root: Option<u64>, pub focus: u64, pub nodes: usize }

/// One node, as much of it as a reader speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeText<'a> { pub id: u64, pub role: u8, pub hidden: bool, pub disabled: bool, pub label: &'a [u8], pub value: &'a [u8] }

struct Bytes<'a> { b: &'a [u8], at: usize }
impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> { let s = self.b.get(self.at..self.at.checked_add(n)?)?; self.at += n; Some(s) }
    fn u8(&mut self) -> Option<u8> { self.take(1).map(|s| s[0]) }
    fn u32(&mut self) -> Option<u32> { self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap())) }
    fn u64(&mut self) -> Option<u64> { self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap())) }
    fn str(&mut self) -> Option<&'a [u8]> { let n = self.u32()? as usize; let s = self.take(n)?; core::str::from_utf8(s).ok()?; Some(s) }
}

/// Reads an encoded update, calling `node` for each node in order. `None` if the bytes
/// are not one whole, valid update; `node` may then have seen some nodes, so a caller
/// that keeps them scans once with a no-op first.
pub fn scan<'a>(bytes: &'a [u8], mut node: impl FnMut(NodeText<'a>)) -> Option<Head> {
    let mut r = Bytes { b: bytes, at: 0 };
    if r.take(4)? != MAGIC || r.u8()? != VERSION { return None; }
    let full = r.u8()? & 1 != 0;
    let root = if full { Some(r.u64()?) } else { None };
    let focus = r.u64()?;
    let count = r.u32()? as usize;
    for _ in 0..count {
        let id = r.u64()?;
        let role = r.u8()?;
        let f = r.u32()?;
        if f & (1 << 10) != 0 { r.take(16)?; }
        let label = r.str()?;
        let value = r.str()?;
        for _ in 0..3 { r.str()?; }
        if f & (1 << 11) != 0 { r.take(32)?; }
        r.take(16)?; // position, set size, level, actions
        let kids = r.u32()? as usize;
        r.take(kids.checked_mul(8)?)?;
        node(NodeText { id, role, hidden: f & 2 != 0, disabled: f & 1 != 0, label, value });
    }
    (r.at == bytes.len()).then_some(Head { full, root, focus, nodes: count })
}

/// How a reader names a role, from AccessKit's `Role` as u8 (accesskit 0.25; Weft's
/// tests check the numbers). Empty for roles a reader does not name (containers).
pub fn role_name(role: u8) -> &'static str {
    match role {
        3 => "",              // Label: its text is the name
        4 => "image",
        5 => "link",
        6 => "row",
        9 => "tree item",
        10 => "list item",    // ListBoxOption
        11 | 12 => "menu item",
        15 => "check box",
        16 => "radio button",
        17 | 31 | 32 | 39 | 42 => "text field",
        18 | 19 => "button",
        25 => "table",
        29 => "switch",
        30 => "menu",
        40 => "password field",
        45 | 66 => "dialog",
        56 | 57 => "combo box",
        87 => "list",         // ListBox
        93 => "menu bar",
        94 => "check menu item",
        95 => "radio menu item",
        101 => "progress bar",
        113 => "slider",
        114 => "spin button",
        116 => "status",
        120 => "tab",
        121 => "tab list",
        128 => "tooltip",
        129 => "tree",
        _ => "",
    }
}

/// What a reader says for a node: "name role", with the value after a text field's
/// or slider's name ("Expression text field 12+30"). Writes into `out`, returns the length.
pub fn speak(node: &NodeText, out: &mut [u8]) -> usize {
    let mut n = 0;
    let mut put = |s: &[u8]| {
        if s.is_empty() { return; }
        if n > 0 && n < out.len() { out[n] = b' '; n += 1; }
        let t = s.len().min(out.len() - n);
        out[n..n + t].copy_from_slice(&s[..t]);
        n += t;
    };
    let (name, value) = if node.label.is_empty() { (node.value, &[][..]) } else { (node.label, node.value) };
    put(name);
    put(role_name(node.role).as_bytes());
    put(value);
    if node.disabled { put(b"dimmed"); }
    n
}
