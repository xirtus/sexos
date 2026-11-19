// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Addresses for devices that arrived below a hot-plug port.
//!
//! Firmware assigned BARs only to what was there at power-on; anything plugged
//! in later (a Thunderbolt dock's switch and controllers, an eGPU enclosure's
//! bridge and GPU) gets its addresses from the port's memory windows here.
//!
//! Nodes form a tree under the port: bridges carry windows for their children,
//! every node may carry memory BARs. Each window and BAR is naturally aligned,
//! bridge windows to at least 1 MiB (the bridge register granularity).
//! Prefetchable BARs go to the prefetchable window when the port has one, else
//! to the ordinary memory window. I/O BARs are not assigned.

pub const MAX_NODES: usize = 32;
// Small on purpose: the kernel plans from the timer tick, on whatever stack it interrupted.
const MAX_ITEMS: usize = 24;
pub const BRIDGE_ALIGN: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    /// Size in bytes, a power of two; 0 = no memory BAR here.
    pub size: u64,
    pub prefetch: bool,
    /// 64-bit BAR (takes the next register too).
    pub wide: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Index of the bridge above, `None` for a node right below the port.
    pub parent: Option<u8>,
    pub bridge: bool,
    pub bars: [Bar; 6],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window { pub base: u64, pub size: u64 }

impl Window {
    pub fn is_open(&self) -> bool { self.size != 0 }
    /// Last byte, as a bridge's limit register wants it.
    pub fn limit(&self) -> u64 { self.base + self.size - 1 }
}

/// What a node got: one address per BAR (0 = none) and, for a bridge, its windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placed { pub bars: [u64; 6], pub mem: Window, pub pref: Window }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// More nodes or BARs under one bridge than this planner holds.
    TooMany,
    /// The port's window is too small: `need` bytes (aligned) against `have`.
    NoRoom { prefetch: bool, need: u64, have: u64 },
    /// A BAR size that is not a power of two, or a parent index out of range.
    Malformed,
}

#[derive(Clone, Copy)]
enum Target { Bar(u8, u8), Window(u8) }

#[derive(Clone, Copy)]
struct Item { size: u64, align: u64, target: Target }

struct Planner<'a> { nodes: &'a [Node], pref_window: bool }

fn align_up(v: u64, a: u64) -> u64 { (v + a - 1) & !(a - 1) }

impl Planner<'_> {
    fn kind(&self, bar: &Bar) -> bool { bar.prefetch && self.pref_window }

    /// Everything that must fit in `parent`'s window of one kind, largest alignment first.
    fn items(&self, parent: Option<u8>, pref: bool, out: &mut [Item; MAX_ITEMS]) -> Result<usize, Error> {
        let mut n = 0;
        for (i, node) in self.nodes.iter().enumerate() {
            if node.parent != parent { continue; }
            for (b, bar) in node.bars.iter().enumerate() {
                if bar.size == 0 || self.kind(bar) != pref { continue; }
                if !bar.size.is_power_of_two() { return Err(Error::Malformed); }
                if n == MAX_ITEMS { return Err(Error::TooMany); }
                out[n] = Item { size: bar.size, align: bar.size, target: Target::Bar(i as u8, b as u8) };
                n += 1;
            }
            if node.bridge {
                let (size, align) = self.need(i as u8, pref)?;
                if size != 0 {
                    if n == MAX_ITEMS { return Err(Error::TooMany); }
                    out[n] = Item { size, align, target: Target::Window(i as u8) };
                    n += 1;
                }
            }
        }
        out[..n].sort_unstable_by(|a, b| b.align.cmp(&a.align));
        Ok(n)
    }

    /// Size and alignment of `bridge`'s window of one kind (0 when nothing below needs it).
    fn need(&self, bridge: u8, pref: bool) -> Result<(u64, u64), Error> {
        let mut items = [Item { size: 0, align: 1, target: Target::Window(0) }; MAX_ITEMS];
        let n = self.items(Some(bridge), pref, &mut items)?;
        if n == 0 { return Ok((0, 0)); }
        let mut end = 0;
        for it in &items[..n] { end = align_up(end, it.align) + it.size; }
        let align = items[0].align.max(BRIDGE_ALIGN);
        Ok((align_up(end, BRIDGE_ALIGN), align))
    }

    fn place(&self, parent: Option<u8>, pref: bool, window: Window, out: &mut [Placed]) -> Result<(), Error> {
        let mut items = [Item { size: 0, align: 1, target: Target::Window(0) }; MAX_ITEMS];
        let n = self.items(parent, pref, &mut items)?;
        let mut at = window.base;
        for it in &items[..n] {
            at = align_up(at, it.align);
            if at + it.size > window.base + window.size {
                let mut need = 0;
                for it in &items[..n] { need = align_up(need, it.align) + it.size; }
                return Err(Error::NoRoom { prefetch: pref, need, have: window.size });
            }
            match it.target {
                Target::Bar(node, bar) => out[node as usize].bars[bar as usize] = at,
                Target::Window(node) => {
                    let w = Window { base: at, size: it.size };
                    if pref { out[node as usize].pref = w } else { out[node as usize].mem = w }
                    self.place(Some(node), pref, w, out)?;
                }
            }
            at += it.size;
        }
        Ok(())
    }
}

/// Plan every BAR and bridge window below a port whose windows are `mem` and
/// `pref` (`pref.size == 0`: the port has no prefetchable window).
pub fn plan(nodes: &[Node], mem: Window, pref: Window, out: &mut [Placed]) -> Result<(), Error> {
    if nodes.len() > MAX_NODES || out.len() < nodes.len() { return Err(Error::TooMany); }
    for node in nodes {
        if let Some(p) = node.parent {
            if p as usize >= nodes.len() || !nodes[p as usize].bridge { return Err(Error::Malformed); }
        }
    }
    // A parent must come before its children, so the tree has no loops.
    for (i, node) in nodes.iter().enumerate() {
        if let Some(p) = node.parent { if p as usize >= i { return Err(Error::Malformed); } }
    }
    for o in out.iter_mut() { *o = Placed::default(); }
    let planner = Planner { nodes, pref_window: pref.is_open() };
    planner.place(None, false, mem, out)?;
    if pref.is_open() { planner.place(None, true, pref, out)?; }
    Ok(())
}

/// Decode a BAR's size from the value read back after writing all ones.
/// `low`/`high` are the original words; returns (size, prefetch, wide) or None
/// for I/O and unimplemented BARs.
pub fn bar_size(low: u32, mask_low: u32, mask_high: u32) -> Option<Bar> {
    if low & 1 != 0 { return None; }
    let wide = (low >> 1) & 3 == 2;
    let mask = if wide { (mask_high as u64) << 32 | (mask_low & !0xf) as u64 } else { 0xffff_ffff_0000_0000 | (mask_low & !0xf) as u64 };
    if mask_low & !0xf == 0 && (!wide || mask_high == 0) { return None; }
    let size = (!mask).wrapping_add(1);
    if size == 0 || !size.is_power_of_two() { return None; }
    Some(Bar { size, prefetch: low & 8 != 0, wide })
}

/// Bridge memory base/limit register (offset 0x20) for a window: 1 MiB units,
/// an empty window encoded as base above limit.
pub fn mem_register(w: Window) -> u32 {
    if !w.is_open() { return 0x0000_fff0; }
    ((w.limit() >> 16) as u32 & 0xfff0) << 16 | (w.base >> 16) as u32 & 0xfff0
}

/// Prefetchable base/limit (0x24) and the upper 32 bits of base (0x28) and limit (0x2c).
pub fn pref_registers(w: Window) -> (u32, u32, u32) {
    if !w.is_open() { return (0x0000_fff1, 0, 0); }
    let low = ((w.limit() >> 16) as u32 & 0xfff0 | 1) << 16 | ((w.base >> 16) as u32 & 0xfff0 | 1);
    (low, (w.base >> 32) as u32, (w.limit() >> 32) as u32)
}

/// A bridge's windows from its registers (0x20, 0x24, 0x28, 0x2c).
pub fn windows(mem: u32, pref: u32, pref_base_hi: u32, pref_limit_hi: u32) -> (Window, Window) {
    let window = |base: u64, limit: u64| if limit < base { Window::default() } else { Window { base, size: limit - base + 1 } };
    // All zeros is a register firmware never set, not a window at address 0.
    let m = if mem == 0 { Window::default() } else { window(((mem & 0xfff0) as u64) << 16, ((mem >> 16 & 0xfff0) as u64) << 16 | 0xf_ffff) };
    let wide = pref & 0xf == 1;
    let pb = ((pref & 0xfff0) as u64) << 16 | if wide { (pref_base_hi as u64) << 32 } else { 0 };
    let pl = ((pref >> 16 & 0xfff0) as u64) << 16 | 0xf_ffff | if wide { (pref_limit_hi as u64) << 32 } else { 0 };
    (m, if pref == 0 { Window::default() } else { window(pb, pl) })
}
