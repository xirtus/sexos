// SPDX-License-Identifier: GPL-3.0-only
//! A copy-on-write filesystem: one checksummed B+tree holds inodes, directory entries
//! and block maps. Nothing committed is ever overwritten. A commit writes new blocks,
//! flushes, then points one of two superblocks at the new root, so a power cut leaves
//! the last commit whole. Free space isn't stored: mounting walks the tree, which
//! removes a whole class of leak and double-use bugs at the price of a slower mount.
#![cfg_attr(not(test), no_std)]

pub const BLOCK: usize = 4096;
pub const NAME_MAX: usize = 255;
pub const ROOT: u64 = 1;
const SB_SLOTS: u64 = 2;
const MAGIC_SB: u64 = 0x3142_5346_5845_53; // "SEXFSB1"
const MAGIC_NODE: u32 = 0x314E_5853; // "SXN1"
const VERSION: u32 = 1;
const MAX_DEPTH: usize = 12;
/// Kept back from file data, so metadata can always finish a commit.
const RESERVED: u64 = 256;
const VALUE_MAX: usize = NAME_MAX + 16;

const K_INODE: u8 = 0;
const K_DIRENT: u8 = 1;
const K_BLOCK: u8 = 2;

const OFF_LEVEL: usize = 4;
const OFF_COUNT: usize = 6;
const OFF_GEN: usize = 8;
const OFF_OWNER: usize = 16;
const OFF_SUM: usize = 24;
const OFF_END: usize = 32;
const OFF_SLOTS: usize = 34;
const KEY_BYTES: usize = 17;
const REC_HEAD: usize = KEY_BYTES + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Io, Corrupt, NoSpace, NotFound, Exists, NotDir, IsDir, NotEmpty, Name, TooBig, Poisoned, Unformatted }
pub type Result<T> = core::result::Result<T, Error>;

pub trait Device {
    fn blocks(&self) -> u64;
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()>;
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> core::result::Result<(), ()>;
    fn flush(&mut self) -> core::result::Result<(), ()>;
    /// Most blocks one `read_run` or `write_run` should carry. A device whose commands
    /// cost the same for one block or many says how many, and file data travels in runs.
    fn run_blocks(&self) -> usize { 1 }
    /// Whole blocks `first..`, as many as `out` holds.
    fn read_run(&mut self, first: u64, out: &mut [u8]) -> core::result::Result<(), ()> {
        for (i, chunk) in out.chunks_exact_mut(BLOCK).enumerate() {
            self.read(first + i as u64, chunk.try_into().map_err(|_| ())?)?;
        }
        Ok(())
    }
    fn write_run(&mut self, first: u64, data: &[u8]) -> core::result::Result<(), ()> {
        for (i, chunk) in data.chunks_exact(BLOCK).enumerate() {
            self.write(first + i as u64, chunk.try_into().map_err(|_| ())?)?;
        }
        Ok(())
    }
}

/// Longest run of file blocks moved together.
const RUN_MAX: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { File, Dir }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat { pub id: u64, pub kind: Kind, pub size: u64, pub mtime: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsStat { pub total: u64, pub free: u64, pub generation: u64 }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Key { id: u64, kind: u8, off: u64 }
impl Key {
    fn bytes(self) -> [u8; KEY_BYTES] {
        let mut b = [0; KEY_BYTES];
        b[..8].copy_from_slice(&self.id.to_be_bytes());
        b[8] = self.kind;
        b[9..].copy_from_slice(&self.off.to_be_bytes());
        b
    }
    fn parse(b: &[u8]) -> Self {
        Key { id: u64::from_be_bytes(b[..8].try_into().unwrap()), kind: b[8], off: u64::from_be_bytes(b[9..17].try_into().unwrap()) }
    }
}

fn get_u16(b: &[u8], at: usize) -> usize { u16::from_le_bytes([b[at], b[at + 1]]) as usize }
fn put_u16(b: &mut [u8], at: usize, v: usize) { b[at..at + 2].copy_from_slice(&(v as u16).to_le_bytes()); }
fn get_u64(b: &[u8], at: usize) -> u64 { u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) }
fn put_u64(b: &mut [u8], at: usize, v: u64) { b[at..at + 8].copy_from_slice(&v.to_le_bytes()); }

/// Word at a time, so checksums stay cheap for whole blocks.
pub fn checksum(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ data.len() as u64;
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0x0000_0100_0000_01B3).rotate_left(29);
    }
    for b in chunks.remainder() { h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01B3); }
    h ^ (h >> 31)
}
fn name_hash(name: &[u8]) -> u64 { checksum(name) >> 1 }

fn node_sum(n: &[u8; BLOCK]) -> u64 {
    let mut h = checksum(&n[..OFF_SUM]);
    h ^= checksum(&n[OFF_SUM + 8..]).rotate_left(17);
    h
}
fn count(n: &[u8]) -> usize { get_u16(n, OFF_COUNT) }
fn level(n: &[u8]) -> u8 { n[OFF_LEVEL] }
fn gen_of(n: &[u8]) -> u64 { get_u64(n, OFF_GEN) }
fn rec_at(n: &[u8], i: usize) -> usize { get_u16(n, OFF_SLOTS + 2 * i) }
fn key_at(n: &[u8], i: usize) -> Key { Key::parse(&n[rec_at(n, i)..]) }
fn val_at(n: &[u8], i: usize) -> &[u8] {
    let at = rec_at(n, i);
    let len = get_u16(n, at + KEY_BYTES);
    &n[at + REC_HEAD..at + REC_HEAD + len]
}
fn node_init(n: &mut [u8; BLOCK], lvl: u8, gen: u64, owner: u64) {
    n.fill(0);
    n[..4].copy_from_slice(&MAGIC_NODE.to_le_bytes());
    n[OFF_LEVEL] = lvl;
    put_u64(n, OFF_GEN, gen);
    put_u64(n, OFF_OWNER, owner);
    put_u16(n, OFF_END, BLOCK);
}
fn free_bytes(n: &[u8]) -> usize { get_u16(n, OFF_END).max(0) - (OFF_SLOTS + 2 * count(n)) }
fn live_bytes(n: &[u8]) -> usize {
    (0..count(n)).map(|i| REC_HEAD + get_u16(n, rec_at(n, i) + KEY_BYTES)).sum()
}
/// Index of the first record with key >= k.
fn lower_bound(n: &[u8], k: Key) -> usize {
    let (mut lo, mut hi) = (0, count(n));
    while lo < hi {
        let mid = (lo + hi) / 2;
        if key_at(n, mid) < k { lo = mid + 1 } else { hi = mid }
    }
    lo
}
fn compact(n: &mut [u8; BLOCK]) {
    let old = *n;
    let c = count(&old);
    let mut end = BLOCK;
    for i in 0..c {
        let at = rec_at(&old, i);
        let len = REC_HEAD + get_u16(&old, at + KEY_BYTES);
        end -= len;
        n[end..end + len].copy_from_slice(&old[at..at + len]);
        put_u16(n, OFF_SLOTS + 2 * i, end);
    }
    put_u16(n, OFF_END, end);
}
fn fits(n: &[u8], val: usize) -> bool {
    REC_HEAD + val + 2 + live_bytes(n) + OFF_SLOTS + 2 * count(n) <= BLOCK
}
/// Caller has checked `fits`.
fn insert_at(n: &mut [u8; BLOCK], i: usize, k: Key, val: &[u8]) {
    let need = REC_HEAD + val.len();
    if free_bytes(n) < need + 2 { compact(n); }
    let c = count(n);
    let end = get_u16(n, OFF_END) - need;
    n[end..end + KEY_BYTES].copy_from_slice(&k.bytes());
    put_u16(n, end + KEY_BYTES, val.len());
    n[end + REC_HEAD..end + need].copy_from_slice(val);
    put_u16(n, OFF_END, end);
    n.copy_within(OFF_SLOTS + 2 * i..OFF_SLOTS + 2 * c, OFF_SLOTS + 2 * i + 2);
    put_u16(n, OFF_SLOTS + 2 * i, end);
    put_u16(n, OFF_COUNT, c + 1);
}
fn remove_at(n: &mut [u8; BLOCK], i: usize) {
    let c = count(n);
    n.copy_within(OFF_SLOTS + 2 * i + 2..OFF_SLOTS + 2 * c, OFF_SLOTS + 2 * i);
    put_u16(n, OFF_COUNT, c - 1);
}
fn child_index(n: &[u8], k: Key) -> usize {
    let i = lower_bound(n, k);
    if i < count(n) && key_at(n, i) == k { i } else { i.saturating_sub(1) }
}
fn child_at(n: &[u8], i: usize) -> u64 { get_u64(val_at(n, i), 0) }
fn set_child(n: &mut [u8; BLOCK], i: usize, block: u64) {
    let at = rec_at(n, i) + REC_HEAD;
    put_u64(n, at, block);
}
fn split_into(left: &mut [u8; BLOCK], right: &mut [u8; BLOCK]) -> Key {
    let c = count(left);
    let total = live_bytes(left);
    let (mut moved, mut cut) = (0, c);
    for i in (1..c).rev() {
        moved += REC_HEAD + get_u16(left, rec_at(left, i) + KEY_BYTES);
        cut = i;
        if moved * 2 >= total { break; }
    }
    for i in cut..c {
        let k = key_at(left, i);
        let v_at = rec_at(left, i) + REC_HEAD;
        let v_len = get_u16(left, rec_at(left, i) + KEY_BYTES);
        let j = count(right);
        let value: [u8; VALUE_MAX] = { let mut b = [0; VALUE_MAX]; b[..v_len].copy_from_slice(&left[v_at..v_at + v_len]); b };
        insert_at(right, j, k, &value[..v_len]);
    }
    put_u16(left, OFF_COUNT, cut);
    compact(left);
    key_at(right, 0)
}

pub struct Slot { block: u64, dirty: bool, stamp: u64, pins: u16 }
impl Slot { pub const EMPTY: Slot = Slot { block: u64::MAX, dirty: false, stamp: 0, pins: 0 }; }

/// All memory comes from the caller, so a service can give it fixed static storage.
pub struct Memory<'a> {
    pub cache: &'a mut [[u8; BLOCK]],
    pub slots: &'a mut [Slot],
    pub used: &'a mut [u64],
    pub pending: &'a mut [u64],
    pub scratch: &'a mut [u8; BLOCK],
}

pub struct Fs<'a, D: Device> {
    dev: D,
    mem: Memory<'a>,
    total: u64,
    root: u64,
    next_id: u64,
    generation: u64,
    tx: u64,
    free: u64,
    pending_count: u64,
    cursor: u64,
    clock: u64,
    changed: bool,
    poisoned: bool,
    value: [u8; VALUE_MAX],
    value_len: usize,
}

fn bit(map: &[u64], i: u64) -> bool { map[(i / 64) as usize] >> (i % 64) & 1 == 1 }
fn set_bit(map: &mut [u64], i: u64, on: bool) {
    let w = &mut map[(i / 64) as usize];
    if on { *w |= 1 << (i % 64) } else { *w &= !(1 << (i % 64)) }
}

impl<'a, D: Device> Fs<'a, D> {
    /// Largest device this memory can track.
    pub fn capacity(mem: &Memory<'_>) -> u64 { (mem.used.len().min(mem.pending.len()) as u64) * 64 }

    /// A fresh filesystem. Destroys whatever the device held.
    pub fn format(mut dev: D, mem: Memory<'a>) -> Result<Self> {
        let total = dev.blocks().min(Self::capacity(&mem));
        if total < RESERVED * 2 { return Err(Error::NoSpace); }
        mem.scratch.fill(0);
        for s in 0..SB_SLOTS { dev.write(s, mem.scratch).map_err(|_| Error::Io)?; }
        dev.flush().map_err(|_| Error::Io)?;
        let mut fs = Self::empty(dev, mem, total);
        fs.reset_maps();
        fs.tx = 1;
        let (leaf, _) = fs.new_node(0)?;
        fs.unpin_all();
        fs.root = leaf;
        fs.next_id = ROOT + 1;
        fs.insert(Key { id: ROOT, kind: K_INODE, off: 0 }, &encode_inode(Kind::Dir, 0, 0))?;
        fs.commit()?;
        Ok(fs)
    }

    pub fn mount(mut dev: D, mem: Memory<'a>) -> Result<Self> {
        let total = dev.blocks().min(Self::capacity(&mem));
        let _ = &mut dev;
        let mut fs = Self::empty(dev, mem, total);
        fs.load()?;
        Ok(fs)
    }

    fn empty(dev: D, mem: Memory<'a>, total: u64) -> Self {
        Fs { dev, mem, total, root: 0, next_id: 0, generation: 0, tx: 0, free: 0, pending_count: 0, cursor: SB_SLOTS, clock: 0,
            changed: false, poisoned: false, value: [0; VALUE_MAX], value_len: 0 }
    }

    fn reset_maps(&mut self) {
        self.mem.used.fill(0);
        self.mem.pending.fill(0);
        for s in 0..SB_SLOTS { set_bit(self.mem.used, s, true); }
        self.free = self.total - SB_SLOTS;
        self.pending_count = 0;
        self.cursor = SB_SLOTS;
    }

    /// Read the newest valid superblock and rebuild free space from the tree.
    fn load(&mut self) -> Result<()> {
        for s in self.mem.slots.iter_mut() { *s = Slot::EMPTY; }
        let mut best: Option<(u64, u64, u64)> = None;
        let mut saw_magic = false;
        for s in 0..SB_SLOTS {
            self.dev.read(s, self.mem.scratch).map_err(|_| Error::Io)?;
            let b = &*self.mem.scratch;
            if get_u64(b, 0) != MAGIC_SB { continue; }
            saw_magic = true;
            if checksum(&b[..56]) != get_u64(b, 56) || get_u64(b, 8) != VERSION as u64 { continue; }
            let (generation, root, next) = (get_u64(b, 16), get_u64(b, 24), get_u64(b, 32));
            if best.map_or(true, |(g, _, _)| generation > g) { best = Some((generation, root, next)); }
        }
        let Some((generation, root, next)) = best else {
            return Err(if saw_magic { Error::Corrupt } else { Error::Unformatted });
        };
        self.generation = generation;
        self.root = root;
        self.next_id = next;
        self.tx = generation + 1;
        self.changed = false;
        self.reset_maps();
        self.walk_mark(root, 0)?;
        self.poisoned = false;
        Ok(())
    }

    fn mark(&mut self, block: u64) -> Result<()> {
        if block < SB_SLOTS || block >= self.total || bit(self.mem.used, block) { return Err(Error::Corrupt); }
        set_bit(self.mem.used, block, true);
        self.free -= 1;
        Ok(())
    }
    fn walk_mark(&mut self, block: u64, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        self.mark(block)?;
        let slot = self.fetch(block)?;
        let n = &self.mem.cache[slot];
        let (lvl, c) = (level(n), count(n));
        for i in 0..c {
            let slot = self.fetch(block)?;
            let n = &self.mem.cache[slot];
            if lvl > 0 {
                let child = child_at(n, i);
                self.walk_mark(child, depth + 1)?;
            } else if key_at(n, i).kind == K_BLOCK {
                let data = get_u64(val_at(n, i), 0);
                self.mark(data)?;
            }
        }
        Ok(())
    }

    // ---- block cache ----
    fn find_slot(&self, block: u64) -> Option<usize> { self.mem.slots.iter().position(|s| s.block == block) }
    fn victim(&mut self) -> Result<usize> {
        let mut best: Option<usize> = None;
        for (i, s) in self.mem.slots.iter().enumerate() {
            if s.pins != 0 { continue; }
            if s.block == u64::MAX { best = Some(i); break; }
            if best.map_or(true, |b| s.stamp < self.mem.slots[b].stamp) { best = Some(i); }
        }
        let i = best.ok_or(Error::Corrupt)?;
        if self.mem.slots[i].dirty { self.write_slot(i)?; }
        self.mem.slots[i] = Slot::EMPTY;
        Ok(i)
    }
    fn write_slot(&mut self, i: usize) -> Result<()> {
        let block = self.mem.slots[i].block;
        let sum = node_sum(&self.mem.cache[i]);
        put_u64(&mut self.mem.cache[i], OFF_SUM, sum);
        self.dev.write(block, &self.mem.cache[i]).map_err(|_| Error::Io)?;
        self.mem.slots[i].dirty = false;
        Ok(())
    }
    fn touch(&mut self, i: usize) { self.clock += 1; self.mem.slots[i].stamp = self.clock; }
    fn fetch(&mut self, block: u64) -> Result<usize> {
        if let Some(i) = self.find_slot(block) { self.touch(i); return Ok(i); }
        if block < SB_SLOTS || block >= self.total { return Err(Error::Corrupt); }
        let i = self.victim()?;
        self.dev.read(block, &mut self.mem.cache[i]).map_err(|_| Error::Io)?;
        let n = &self.mem.cache[i];
        if u32::from_le_bytes(n[..4].try_into().unwrap()) != MAGIC_NODE || get_u64(n, OFF_OWNER) != block
            || get_u64(n, OFF_SUM) != node_sum(n) { return Err(Error::Corrupt); }
        self.mem.slots[i] = Slot { block, dirty: false, stamp: 0, pins: 0 };
        self.touch(i);
        Ok(i)
    }
    fn pin(&mut self, i: usize) { self.mem.slots[i].pins += 1; }
    fn unpin(&mut self, i: usize) { self.mem.slots[i].pins -= 1; }
    fn unpin_all(&mut self) { for s in self.mem.slots.iter_mut() { s.pins = 0; } }

    fn alloc(&mut self) -> Result<u64> {
        if self.free == 0 { return Err(Error::NoSpace); }
        for _ in 0..2 {
            let mut at = self.cursor;
            while at < self.total {
                let word = self.mem.used[(at / 64) as usize];
                if word == u64::MAX { at = (at / 64 + 1) * 64; continue; }
                if !bit(self.mem.used, at) {
                    set_bit(self.mem.used, at, true);
                    self.free -= 1;
                    self.cursor = at + 1;
                    return Ok(at);
                }
                at += 1;
            }
            self.cursor = SB_SLOTS;
        }
        Err(Error::NoSpace)
    }
    /// Takes exactly `block` if it is free, to grow a run of file data.
    fn take(&mut self, block: u64) -> bool {
        if block >= self.total || self.free <= RESERVED || bit(self.mem.used, block) { return false; }
        set_bit(self.mem.used, block, true);
        self.free -= 1;
        self.cursor = block + 1;
        true
    }
    /// Free once the commit that stops using it is on disk.
    fn release(&mut self, block: u64) {
        if !bit(self.mem.pending, block) { set_bit(self.mem.pending, block, true); self.pending_count += 1; }
    }
    fn new_node(&mut self, lvl: u8) -> Result<(u64, usize)> {
        let block = self.alloc()?;
        let i = self.victim()?;
        let tx = self.tx;
        node_init(&mut self.mem.cache[i], lvl, tx, block);
        self.mem.slots[i] = Slot { block, dirty: true, stamp: 0, pins: 0 };
        self.touch(i);
        Ok((block, i))
    }
    /// The node as this transaction may change it: itself if already this
    /// transaction's, otherwise a copy at a new block.
    fn writable(&mut self, block: u64) -> Result<(u64, usize)> {
        let i = self.fetch(block)?;
        if gen_of(&self.mem.cache[i]) == self.tx { self.mem.slots[i].dirty = true; return Ok((block, i)); }
        self.pin(i);
        let copy = self.alloc();
        self.unpin(i);
        let copy = copy?;
        self.release(block);
        let tx = self.tx;
        let n = &mut self.mem.cache[i];
        put_u64(n, OFF_GEN, tx);
        put_u64(n, OFF_OWNER, copy);
        self.mem.slots[i].block = copy;
        self.mem.slots[i].dirty = true;
        Ok((copy, i))
    }
    fn drop_node(&mut self, block: u64, i: usize) {
        let mine = gen_of(&self.mem.cache[i]) == self.tx;
        self.mem.slots[i] = Slot::EMPTY;
        if mine { set_bit(self.mem.used, block, false); self.free += 1; } else { self.release(block); }
    }

    // ---- tree ----
    fn insert(&mut self, k: Key, val: &[u8]) -> Result<()> {
        self.changed = true;
        let root = self.root;
        let result = self.insert_in(root, k, val, 0);
        self.unpin_all();
        let (new_root, split) = result?;
        self.root = new_root;
        if let Some((sk, right)) = split {
            let (block, i) = self.new_node(0)?;
            let child_level = { let c = self.fetch(new_root)?; level(&self.mem.cache[c]) };
            let i = self.find_slot(block).unwrap_or(i);
            let n = &mut self.mem.cache[i];
            n[OFF_LEVEL] = child_level + 1;
            insert_at(n, 0, Key { id: 0, kind: 0, off: 0 }, &new_root.to_le_bytes());
            insert_at(n, 1, sk, &right.to_le_bytes());
            self.root = block;
        }
        Ok(())
    }
    fn insert_in(&mut self, block: u64, k: Key, val: &[u8], depth: usize) -> Result<(u64, Option<(Key, u64)>)> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        let (new_block, i) = self.writable(block)?;
        self.pin(i);
        if level(&self.mem.cache[i]) == 0 {
            let n = &mut self.mem.cache[i];
            let at = lower_bound(n, k);
            if at < count(n) && key_at(n, at) == k { remove_at(n, at); }
            let split = self.place(i, k, val)?;
            self.unpin(i);
            return Ok((new_block, split));
        }
        let at = child_index(&self.mem.cache[i], k);
        let child = child_at(&self.mem.cache[i], at);
        let (new_child, child_split) = self.insert_in(child, k, val, depth + 1)?;
        set_child(&mut self.mem.cache[i], at, new_child);
        self.mem.slots[i].dirty = true;
        let split = match child_split {
            Some((sk, right)) => self.place(i, sk, &right.to_le_bytes())?,
            None => None,
        };
        self.unpin(i);
        Ok((new_block, split))
    }
    /// Put a record in a pinned node, splitting it if needed.
    fn place(&mut self, i: usize, k: Key, val: &[u8]) -> Result<Option<(Key, u64)>> {
        self.mem.slots[i].dirty = true;
        if fits(&self.mem.cache[i], val.len()) {
            let at = lower_bound(&self.mem.cache[i], k);
            insert_at(&mut self.mem.cache[i], at, k, val);
            return Ok(None);
        }
        let lvl = level(&self.mem.cache[i]);
        let (right_block, r) = self.new_node(lvl)?;
        self.pin(r);
        let (a, b) = two_mut(self.mem.cache, i, r);
        let sk = split_into(a, b);
        let target = if k < sk { i } else { r };
        let at = lower_bound(&self.mem.cache[target], k);
        insert_at(&mut self.mem.cache[target], at, k, val);
        self.unpin(r);
        Ok(Some((sk, right_block)))
    }
    fn remove(&mut self, k: Key) -> Result<()> {
        self.changed = true;
        let root = self.root;
        let result = self.remove_in(root, k, 0);
        self.unpin_all();
        let (mut new_root, empty) = result?;
        if empty {
            let (block, _) = self.new_node(0)?;
            new_root = block;
        }
        // Collapse a root with one child.
        loop {
            let i = self.fetch(new_root)?;
            let n = &self.mem.cache[i];
            if level(n) == 0 || count(n) != 1 { break; }
            let child = child_at(n, 0);
            self.drop_node(new_root, i);
            new_root = child;
        }
        self.root = new_root;
        Ok(())
    }
    fn remove_in(&mut self, block: u64, k: Key, depth: usize) -> Result<(u64, bool)> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        let (new_block, i) = self.writable(block)?;
        self.pin(i);
        if level(&self.mem.cache[i]) == 0 {
            let n = &mut self.mem.cache[i];
            let at = lower_bound(n, k);
            if at >= count(n) || key_at(n, at) != k { return Err(Error::NotFound); }
            remove_at(n, at);
            self.unpin(i);
            if count(&self.mem.cache[i]) == 0 { self.drop_node(new_block, i); return Ok((new_block, true)); }
            return Ok((new_block, false));
        }
        let at = child_index(&self.mem.cache[i], k);
        let child = child_at(&self.mem.cache[i], at);
        let (new_child, empty) = self.remove_in(child, k, depth + 1)?;
        if empty { remove_at(&mut self.mem.cache[i], at); } else { set_child(&mut self.mem.cache[i], at, new_child); }
        self.unpin(i);
        if count(&self.mem.cache[i]) == 0 { self.drop_node(new_block, i); return Ok((new_block, true)); }
        Ok((new_block, false))
    }
    /// The first record at or after `k`, with its value in `self.value`.
    fn seek(&mut self, k: Key) -> Result<Option<Key>> { let root = self.root; self.seek_in(root, k, 0) }
    fn seek_in(&mut self, block: u64, k: Key, depth: usize) -> Result<Option<Key>> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        let i = self.fetch(block)?;
        let n = &self.mem.cache[i];
        if level(n) == 0 {
            let at = lower_bound(n, k);
            if at >= count(n) { return Ok(None); }
            let found = key_at(n, at);
            let v = val_at(n, at);
            self.value_len = v.len();
            self.value[..v.len()].copy_from_slice(v);
            return Ok(Some(found));
        }
        let (start, c) = (child_index(n, k), count(n));
        for at in start..c {
            let i = self.fetch(block)?;
            let child = child_at(&self.mem.cache[i], at);
            if let Some(found) = self.seek_in(child, k, depth + 1)? { return Ok(Some(found)); }
        }
        Ok(None)
    }
    fn get(&mut self, k: Key) -> Result<bool> { Ok(self.seek(k)? == Some(k)) }

    // ---- files ----
    fn check(&self) -> Result<()> { if self.poisoned { Err(Error::Poisoned) } else { Ok(()) } }
    fn guarded<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.check()?;
        let result = f(self);
        if matches!(result, Err(Error::Io | Error::Corrupt)) { self.poisoned = true; }
        result
    }
    fn inode(&mut self, id: u64) -> Result<(Kind, u64, u64)> {
        if !self.get(Key { id, kind: K_INODE, off: 0 })? { return Err(Error::NotFound); }
        decode_inode(&self.value[..self.value_len])
    }
    fn lookup_in(&mut self, dir: u64, name: &[u8]) -> Result<Option<(u64, u64)>> {
        let mut off = name_hash(name);
        loop {
            let k = Key { id: dir, kind: K_DIRENT, off };
            if !self.get(k)? { return Ok(None); }
            if &self.value[9..self.value_len] == name { return Ok(Some((get_u64(&self.value, 0), off))); }
            off = off.wrapping_add(1);
        }
    }
    fn valid_name(name: &[u8]) -> Result<()> {
        if name.is_empty() || name.len() > NAME_MAX || name == b"." || name == b".." || name.iter().any(|b| *b == b'/' || *b == 0) { return Err(Error::Name); }
        Ok(())
    }
    /// The directory holding the last part of `path`, and that part.
    fn parent<'p>(&mut self, path: &'p [u8]) -> Result<(u64, &'p [u8])> {
        let trimmed = trim(path);
        if trimmed.is_empty() { return Err(Error::Name); }
        let (head, name) = match trimmed.iter().rposition(|b| *b == b'/') {
            Some(i) => (&trimmed[..i], &trimmed[i + 1..]),
            None => (&trimmed[..0], trimmed),
        };
        Self::valid_name(name)?;
        let dir = self.walk(head)?;
        if self.inode(dir)?.0 != Kind::Dir { return Err(Error::NotDir); }
        Ok((dir, name))
    }
    fn walk(&mut self, path: &[u8]) -> Result<u64> {
        let mut at = ROOT;
        for part in trim(path).split(|b| *b == b'/').filter(|p| !p.is_empty()) {
            if self.inode(at)?.0 != Kind::Dir { return Err(Error::NotDir); }
            at = self.lookup_in(at, part)?.ok_or(Error::NotFound)?.0;
        }
        Ok(at)
    }
    fn add_entry(&mut self, dir: u64, name: &[u8], child: u64, kind: Kind) -> Result<()> {
        let mut off = name_hash(name);
        while self.get(Key { id: dir, kind: K_DIRENT, off })? { off = off.wrapping_add(1); }
        let mut v = [0u8; VALUE_MAX];
        put_u64(&mut v, 0, child);
        v[8] = (kind == Kind::Dir) as u8;
        v[9..9 + name.len()].copy_from_slice(name);
        self.insert(Key { id: dir, kind: K_DIRENT, off }, &v[..9 + name.len()])
    }
    /// Removing a hash-probed entry must not break the chain behind it: later entries
    /// move up, so a lookup stops only at a true gap.
    fn drop_entry(&mut self, dir: u64, mut off: u64) -> Result<()> {
        self.remove(Key { id: dir, kind: K_DIRENT, off })?;
        loop {
            let next = off.wrapping_add(1);
            if !self.get(Key { id: dir, kind: K_DIRENT, off: next })? { return Ok(()); }
            let len = self.value_len;
            let mut v = [0u8; VALUE_MAX];
            v[..len].copy_from_slice(&self.value[..len]);
            let home = name_hash(&v[9..len]);
            if home > off { return Ok(()); }
            self.remove(Key { id: dir, kind: K_DIRENT, off: next })?;
            self.insert(Key { id: dir, kind: K_DIRENT, off }, &v[..len])?;
            off = next;
        }
    }
    fn create_at(&mut self, path: &[u8], kind: Kind, now: u64) -> Result<u64> {
        let (dir, name) = self.parent(path)?;
        if self.lookup_in(dir, name)?.is_some() { return Err(Error::Exists); }
        if self.free <= RESERVED { return Err(Error::NoSpace); }
        let id = self.next_id;
        self.next_id += 1;
        self.insert(Key { id, kind: K_INODE, off: 0 }, &encode_inode(kind, 0, now))?;
        self.add_entry(dir, name, id, kind)?;
        Ok(id)
    }
    pub fn create(&mut self, path: &[u8], now: u64) -> Result<u64> { self.guarded(|f| f.create_at(path, Kind::File, now)) }
    pub fn mkdir(&mut self, path: &[u8], now: u64) -> Result<u64> { self.guarded(|f| f.create_at(path, Kind::Dir, now)) }
    pub fn open(&mut self, path: &[u8]) -> Result<Stat> {
        self.guarded(|f| { let id = f.walk(path)?; let (kind, size, mtime) = f.inode(id)?; Ok(Stat { id, kind, size, mtime }) })
    }
    pub fn stat(&mut self, id: u64) -> Result<Stat> {
        self.guarded(|f| { let (kind, size, mtime) = f.inode(id)?; Ok(Stat { id, kind, size, mtime }) })
    }
    pub fn write(&mut self, id: u64, offset: u64, data: &[u8], now: u64) -> Result<usize> {
        self.guarded(|f| f.write_inner(id, offset, data, now))
    }
    fn write_inner(&mut self, id: u64, offset: u64, data: &[u8], now: u64) -> Result<usize> {
        let (kind, size, _) = self.inode(id)?;
        if kind != Kind::File { return Err(Error::IsDir); }
        offset.checked_add(data.len() as u64).ok_or(Error::TooBig)?;
        if data.is_empty() { return Ok(0); }
        let (mut done, mut at) = (0usize, offset);
        while done < data.len() {
            if self.free <= RESERVED { break; }
            match self.write_step(id, at, &data[done..]) {
                Ok(n) => { done += n; at += n as u64; }
                // After progress, a full disk or tree ends the write short; the prefix counts.
                Err(e) if done > 0 && !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) => break,
                Err(e) => return Err(e),
            }
        }
        if done == 0 && !data.is_empty() { return Err(Error::NoSpace); }
        let new_size = size.max(offset + done as u64);
        self.insert(Key { id, kind: K_INODE, off: 0 }, &encode_inode(Kind::File, new_size, now))?;
        Ok(done)
    }
    /// Writes the next block, or the next run of whole blocks, of `data` at file offset `at`.
    fn write_step(&mut self, id: u64, at: u64, data: &[u8]) -> Result<usize> {
        let index = at / BLOCK as u64;
        let inside = (at % BLOCK as u64) as usize;
        let whole = if inside == 0 { data.len() / BLOCK } else { 0 };
        let want = whole.min(self.dev.run_blocks()).min(RUN_MAX);
        if want >= 2 {
            // New blocks side by side, so the device takes them in one command.
            let first = self.alloc()?;
            let mut n = 1;
            while n < want && self.take(first + n as u64) { n += 1; }
            self.dev.write_run(first, &data[..n * BLOCK]).map_err(|_| Error::Io)?;
            for j in 0..n {
                let sum = checksum(&data[j * BLOCK..(j + 1) * BLOCK]);
                self.map_block(id, index + j as u64, first + j as u64, sum)?;
            }
            return Ok(n * BLOCK);
        }
        let take = (BLOCK - inside).min(data.len());
        if take == BLOCK {
            self.mem.scratch.copy_from_slice(&data[..BLOCK]);
        } else if !self.read_block(id, index)? {
            self.mem.scratch.fill(0);
        }
        self.mem.scratch[inside..inside + take].copy_from_slice(&data[..take]);
        let sum = checksum(&self.mem.scratch[..]);
        let block = self.alloc()?;
        self.dev.write(block, self.mem.scratch).map_err(|_| Error::Io)?;
        self.map_block(id, index, block, sum)?;
        Ok(take)
    }
    fn map_block(&mut self, id: u64, index: u64, block: u64, sum: u64) -> Result<()> {
        let k = Key { id, kind: K_BLOCK, off: index };
        if self.get(k)? { let old = get_u64(&self.value, 0); self.release(old); }
        let mut v = [0u8; 16];
        put_u64(&mut v, 0, block);
        put_u64(&mut v, 8, sum);
        self.insert(k, &v)
    }
    /// Reads a file block into scratch. False if it is a hole.
    fn read_block(&mut self, id: u64, index: u64) -> Result<bool> {
        if !self.get(Key { id, kind: K_BLOCK, off: index })? { return Ok(false); }
        let (block, sum) = (get_u64(&self.value, 0), get_u64(&self.value, 8));
        self.dev.read(block, self.mem.scratch).map_err(|_| Error::Io)?;
        if checksum(&self.mem.scratch[..]) != sum { return Err(Error::Corrupt); }
        Ok(true)
    }
    pub fn read(&mut self, id: u64, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.guarded(|f| {
            let (kind, size, _) = f.inode(id)?;
            if kind != Kind::File { return Err(Error::IsDir); }
            if offset >= size { return Ok(0); }
            let want = (out.len() as u64).min(size - offset) as usize;
            let (mut done, mut at) = (0usize, offset);
            while done < want {
                let inside = (at % BLOCK as u64) as usize;
                if inside == 0 {
                    let n = f.read_run(id, at / BLOCK as u64, &mut out[done..want])?;
                    if n > 0 { done += n * BLOCK; at += (n * BLOCK) as u64; continue; }
                }
                let take = (BLOCK - inside).min(want - done);
                if f.read_block(id, at / BLOCK as u64)? {
                    out[done..done + take].copy_from_slice(&f.mem.scratch[inside..inside + take]);
                } else {
                    out[done..done + take].fill(0);
                }
                done += take;
                at += take as u64;
            }
            Ok(done)
        })
    }
    /// Whole file blocks from `index` that sit side by side on disk, read with one device
    /// run straight into `out`. Returns how many; 0 when fewer than two line up.
    fn read_run(&mut self, id: u64, index: u64, out: &mut [u8]) -> Result<usize> {
        let want = (out.len() / BLOCK).min(self.dev.run_blocks()).min(RUN_MAX);
        if want < 2 { return Ok(0); }
        let mut sums = [0u64; RUN_MAX];
        let (mut first, mut n) = (0u64, 0usize);
        while n < want {
            if !self.get(Key { id, kind: K_BLOCK, off: index + n as u64 })? { break; }
            let block = get_u64(&self.value, 0);
            if n == 0 { first = block; } else if block != first + n as u64 { break; }
            if block < SB_SLOTS || block >= self.total { return Err(Error::Corrupt); }
            sums[n] = get_u64(&self.value, 8);
            n += 1;
        }
        if n < 2 { return Ok(0); }
        let run = &mut out[..n * BLOCK];
        self.dev.read_run(first, run).map_err(|_| Error::Io)?;
        for j in 0..n {
            if checksum(&run[j * BLOCK..(j + 1) * BLOCK]) != sums[j] { return Err(Error::Corrupt); }
        }
        Ok(n)
    }
    pub fn truncate(&mut self, id: u64, size: u64, now: u64) -> Result<()> {
        self.guarded(|f| {
            let (kind, old, _) = f.inode(id)?;
            if kind != Kind::File { return Err(Error::IsDir); }
            if size < old {
                let keep = size.div_ceil(BLOCK as u64);
                f.drop_blocks(id, keep)?;
                if size % BLOCK as u64 != 0 && f.read_block(id, size / BLOCK as u64)? {
                    let inside = (size % BLOCK as u64) as usize;
                    let (mut tail, index) = ([0u8; 1], size / BLOCK as u64);
                    let _ = &mut tail;
                    f.mem.scratch[inside..].fill(0);
                    let sum = checksum(&f.mem.scratch[..]);
                    let block = f.alloc()?;
                    f.dev.write(block, f.mem.scratch).map_err(|_| Error::Io)?;
                    let k = Key { id, kind: K_BLOCK, off: index };
                    if f.get(k)? { let old = get_u64(&f.value, 0); f.release(old); }
                    let mut v = [0u8; 16];
                    put_u64(&mut v, 0, block);
                    put_u64(&mut v, 8, sum);
                    f.insert(k, &v)?;
                }
            }
            f.insert(Key { id, kind: K_INODE, off: 0 }, &encode_inode(Kind::File, size, now))
        })
    }
    fn drop_blocks(&mut self, id: u64, from: u64) -> Result<()> {
        loop {
            let Some(k) = self.seek(Key { id, kind: K_BLOCK, off: from })? else { return Ok(()) };
            if k.id != id || k.kind != K_BLOCK { return Ok(()); }
            let old = get_u64(&self.value, 0);
            self.release(old);
            self.remove(k)?;
        }
    }
    fn dir_is_empty(&mut self, id: u64) -> Result<bool> {
        Ok(!matches!(self.seek(Key { id, kind: K_DIRENT, off: 0 })?, Some(k) if k.id == id && k.kind == K_DIRENT))
    }
    pub fn remove_path(&mut self, path: &[u8]) -> Result<()> {
        self.guarded(|f| {
            let (dir, name) = f.parent(path)?;
            let (id, off) = f.lookup_in(dir, name)?.ok_or(Error::NotFound)?;
            let (kind, _, _) = f.inode(id)?;
            if kind == Kind::Dir && !f.dir_is_empty(id)? { return Err(Error::NotEmpty); }
            f.drop_entry(dir, off)?;
            f.drop_blocks(id, 0)?;
            f.remove(Key { id, kind: K_INODE, off: 0 })
        })
    }
    /// Atomic at the next commit. A file already at `to` is replaced.
    pub fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<()> {
        self.guarded(|f| {
            let (src_dir, src_name) = f.parent(from)?;
            let (id, src_off) = f.lookup_in(src_dir, src_name)?.ok_or(Error::NotFound)?;
            let (kind, _, _) = f.inode(id)?;
            let (dst_dir, dst_name) = f.parent(to)?;
            if kind == Kind::Dir {
                // A directory can't move inside itself.
                let mut at = dst_dir;
                loop {
                    if at == id { return Err(Error::Name); }
                    if at == ROOT { break; }
                    at = f.parent_of(at)?;
                }
            }
            if let Some((other, off)) = f.lookup_in(dst_dir, dst_name)? {
                if other == id { return Ok(()); }
                let (other_kind, _, _) = f.inode(other)?;
                if other_kind == Kind::Dir { return Err(Error::IsDir); }
                if kind == Kind::Dir { return Err(Error::NotDir); }
                f.drop_entry(dst_dir, off)?;
                f.drop_blocks(other, 0)?;
                f.remove(Key { id: other, kind: K_INODE, off: 0 })?;
            }
            // The entry may have moved up if the drop above touched its chain.
            let (_, src_off2) = f.lookup_in(src_dir, src_name)?.ok_or(Error::Corrupt)?;
            let _ = src_off;
            f.drop_entry(src_dir, src_off2)?;
            f.add_entry(dst_dir, dst_name, id, kind)
        })
    }
    /// Directories hold no parent link, so this searches. Only a directory rename uses it.
    fn parent_of(&mut self, id: u64) -> Result<u64> {
        let mut dir = ROOT;
        let mut stack = [0u64; 64];
        let mut depth = 0;
        loop {
            let mut off = 0;
            while let Some(k) = self.seek(Key { id: dir, kind: K_DIRENT, off })? {
                if k.id != dir || k.kind != K_DIRENT { break; }
                let child = get_u64(&self.value, 0);
                if child == id { return Ok(dir); }
                if self.value[8] == 1 && depth < stack.len() { stack[depth] = child; depth += 1; }
                off = k.off + 1;
            }
            if depth == 0 { return Err(Error::Corrupt); }
            depth -= 1;
            dir = stack[depth];
        }
    }
    /// One directory entry at or after `cursor`. Returns the name length, kind, id and
    /// the next cursor.
    pub fn read_dir(&mut self, dir: u64, cursor: u64, name: &mut [u8; NAME_MAX]) -> Result<Option<(usize, Kind, u64, u64)>> {
        self.guarded(|f| {
            if f.inode(dir)?.0 != Kind::Dir { return Err(Error::NotDir); }
            let Some(k) = f.seek(Key { id: dir, kind: K_DIRENT, off: cursor })? else { return Ok(None) };
            if k.id != dir || k.kind != K_DIRENT { return Ok(None); }
            let len = f.value_len - 9;
            name[..len].copy_from_slice(&f.value[9..f.value_len]);
            let kind = if f.value[8] == 1 { Kind::Dir } else { Kind::File };
            Ok(Some((len, kind, get_u64(&f.value, 0), k.off + 1)))
        })
    }
    pub fn statfs(&self) -> FsStat { FsStat { total: self.total, free: self.free + self.pending_count, generation: self.generation } }
    pub fn has_changes(&self) -> bool { self.changed }

    /// Makes everything so far durable, all together. Returns once it is.
    pub fn commit(&mut self) -> Result<()> {
        self.check()?;
        if !self.changed && self.generation != 0 { return Ok(()); }
        let result = self.commit_inner();
        if result.is_err() { self.poisoned = true; }
        result
    }
    fn commit_inner(&mut self) -> Result<()> {
        for i in 0..self.mem.slots.len() {
            if self.mem.slots[i].dirty { self.write_slot(i)?; }
        }
        self.dev.flush().map_err(|_| Error::Io)?;
        let generation = self.generation + 1;
        let b = &mut *self.mem.scratch;
        b.fill(0);
        put_u64(b, 0, MAGIC_SB);
        put_u64(b, 8, VERSION as u64);
        put_u64(b, 16, generation);
        put_u64(b, 24, self.root);
        put_u64(b, 32, self.next_id);
        put_u64(b, 40, self.total);
        let sum = checksum(&b[..56]);
        put_u64(b, 56, sum);
        self.dev.write(generation % SB_SLOTS, self.mem.scratch).map_err(|_| Error::Io)?;
        self.dev.flush().map_err(|_| Error::Io)?;
        self.generation = generation;
        self.tx = generation + 1;
        self.changed = false;
        // Only now can what the old commit used be reused.
        for w in 0..self.mem.pending.len() {
            let bits = self.mem.pending[w];
            if bits != 0 {
                self.mem.used[w] &= !bits;
                self.mem.pending[w] = 0;
            }
        }
        self.free += self.pending_count;
        self.pending_count = 0;
        Ok(())
    }
    /// Forget everything since the last commit, including after an error.
    pub fn rollback(&mut self) -> Result<()> { self.load() }
    pub fn device(&mut self) -> &mut D { &mut self.dev }
    pub fn into_parts(self) -> (D, Memory<'a>) { (self.dev, self.mem) }

    /// Every invariant, for tests and for a repair command later. Returns (nodes, data blocks).
    pub fn verify(&mut self) -> Result<(u64, u64)> {
        let root = self.root;
        let (mut nodes, mut data) = (0, 0);
        let mut last = None;
        self.verify_in(root, 0, None, &mut last, &mut nodes, &mut data)?;
        Ok((nodes, data))
    }
    fn verify_in(&mut self, block: u64, depth: usize, expect: Option<u8>, last: &mut Option<Key>, nodes: &mut u64, data: &mut u64) -> Result<()> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        *nodes += 1;
        if !bit(self.mem.used, block) { return Err(Error::Corrupt); }
        let i = self.fetch(block)?;
        let n = &self.mem.cache[i];
        let (lvl, c) = (level(n), count(n));
        if expect.is_some_and(|e| e != lvl) { return Err(Error::Corrupt); }
        for at in 0..c {
            let i = self.fetch(block)?;
            let n = &self.mem.cache[i];
            let k = key_at(n, at);
            if lvl == 0 {
                if last.is_some_and(|l| l >= k) { return Err(Error::Corrupt); }
                *last = Some(k);
                if k.kind == K_BLOCK {
                    *data += 1;
                    if !bit(self.mem.used, get_u64(val_at(n, at), 0)) { return Err(Error::Corrupt); }
                }
            } else {
                let child = child_at(n, at);
                self.verify_in(child, depth + 1, Some(lvl - 1), last, nodes, data)?;
            }
        }
        Ok(())
    }
}

fn two_mut(cache: &mut [[u8; BLOCK]], a: usize, b: usize) -> (&mut [u8; BLOCK], &mut [u8; BLOCK]) {
    if a < b { let (l, r) = cache.split_at_mut(b); (&mut l[a], &mut r[0]) }
    else { let (l, r) = cache.split_at_mut(a); (&mut r[0], &mut l[b]) }
}
fn trim(path: &[u8]) -> &[u8] {
    let start = path.iter().position(|b| *b != b'/').unwrap_or(path.len());
    let end = path.iter().rposition(|b| *b != b'/').map_or(start, |i| i + 1);
    &path[start..end.max(start)]
}
fn encode_inode(kind: Kind, size: u64, mtime: u64) -> [u8; 17] {
    let mut v = [0; 17];
    v[0] = (kind == Kind::Dir) as u8;
    v[1..9].copy_from_slice(&size.to_le_bytes());
    v[9..17].copy_from_slice(&mtime.to_le_bytes());
    v
}
fn decode_inode(v: &[u8]) -> Result<(Kind, u64, u64)> {
    if v.len() != 17 { return Err(Error::Corrupt); }
    Ok((if v[0] == 1 { Kind::Dir } else { Kind::File }, get_u64(v, 1), get_u64(v, 9)))
}
