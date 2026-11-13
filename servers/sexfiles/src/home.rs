// SPDX-License-Identifier: GPL-3.0-only
//! The encrypted home. `/home` is a SexOS filesystem of its own, kept in one file of the
//! system disk (`/.home.sexcrypt`) and encrypted block by block with XTS-AES-256 under a
//! random 512-bit key; the key is stored only wrapped by the person's password through
//! Argon2id (`sex-crypt`). Until the password is given, `/home` is an empty folder and
//! every path under it answers LOCKED: nothing in it can be read, by Files or anyone.
//!
//! Inside, it is sexfs as on the system disk: copy-on-write, checksummed, with its own
//! commits. Its blocks are written into the file through the system filesystem, and its
//! flush commits that one too, so a power cut leaves both whole. Locking writes it out,
//! forgets the key, and ends every grant into `/home`.
//!
//! The key lives in Files' memory while home is unlocked, as the plaintext it serves does:
//! Files is the storage server that already sees every file, and the kernel never holds
//! either.
use crate::fs_service::Disk as FsDisk;
use alloc::vec;
use sex_crypt::{Cost, Xts, HEADER, KEY};
use sex_pdx::fs as w;
use sex_pdx::home_lock as h;
use sexfs::{Device, Error, Fs, Memory, Slot, BLOCK};

/// The system filesystem, lent for the length of one request on `/home`.
static mut MAIN: *mut Fs<'static, FsDisk> = core::ptr::null_mut();
static mut HOME: Option<Fs<'static, HomeDev>> = None;
/// Blocks the home filesystem keeps in memory.
const CACHE: usize = 256;

pub struct HomeDev { id: u64, blocks: u64, xts: Xts }
impl HomeDev {
    fn main() -> Result<&'static mut Fs<'static, FsDisk>, ()> {
        let p = unsafe { MAIN };
        if p.is_null() { return Err(()); }
        Ok(unsafe { &mut *p })
    }
    /// Data block `n` sits after the header block.
    fn at(n: u64) -> u64 { (n + 1) * BLOCK as u64 }
}
impl Device for HomeDev {
    fn blocks(&self) -> u64 { self.blocks }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        let fs = Self::main()?;
        let mut done = 0;
        while done < BLOCK {
            match fs.read(self.id, Self::at(block) + done as u64, &mut out[done..]) {
                Ok(0) => { out[done..].fill(0); break; }
                Ok(n) => done += n,
                Err(_) => return Err(()),
            }
        }
        // A block never written reads as zeros, as on a fresh disk.
        if out.iter().all(|b| *b == 0) { return Ok(()); }
        self.xts.decrypt(block, out);
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        let fs = Self::main()?;
        let mut sealed = *data;
        self.xts.encrypt(block, &mut sealed);
        let mut done = 0;
        while done < BLOCK {
            match fs.write(self.id, Self::at(block) + done as u64, &sealed[done..], 0) {
                Ok(0) | Err(_) => return Err(()),
                Ok(n) => done += n,
            }
        }
        crate::fs_service::wrote(BLOCK as u64);
        Ok(())
    }
    /// Home's commit is durable once the system filesystem's is.
    fn flush(&mut self) -> Result<(), ()> { Self::main()?.commit().map_err(|_| ()) }
}

fn memory(blocks: u64) -> Memory<'static> {
    let words = (blocks as usize).div_ceil(64) + 1;
    Memory {
        cache: vec![[0u8; BLOCK]; CACHE].leak(),
        slots: vec![Slot::EMPTY; CACHE].leak(),
        used: vec![0u64; words].leak(),
        pending: vec![0u64; words].leak(),
        scratch: alloc::boxed::Box::leak(alloc::boxed::Box::new([0u8; BLOCK])),
    }
}

/// Random bytes from the CPU (RDRAND), or None on a CPU without it.
fn random(out: &mut [u8]) -> Option<()> {
    let ecx = core::arch::x86_64::__cpuid(1).ecx;
    if ecx & (1 << 30) == 0 { return None; }
    for chunk in out.chunks_mut(8) {
        let mut word = 0u64;
        let mut ok = 0u8;
        for _ in 0..32 {
            unsafe { core::arch::asm!("rdrand {w}", "setc {c}", w = out(reg) word, c = out(reg_byte) ok, options(nomem, nostack)); }
            if ok == 1 { break; }
        }
        if ok != 1 { return None; }
        chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
    }
    Some(())
}

/// Runs `f` with the system filesystem lent to home's device.
fn lend<T>(main: &mut Fs<'static, FsDisk>, f: impl FnOnce() -> T) -> T {
    unsafe { MAIN = main; }
    let out = f();
    unsafe { MAIN = core::ptr::null_mut(); }
    out
}
/// Work on home runs on its own stack. Files' own stack is small and already deep when a
/// request arrives; the key derivation, and a write that passes through two file systems
/// (home's, then the system disk's), need more than is left.
const DEEP: usize = 256 * 1024;
static mut DEEP_STACK: *mut u8 = core::ptr::null_mut();
static mut DEEP_IN: bool = false;
fn deep<R>(f: impl FnOnce() -> R) -> R {
    // Already on it: one stack, so no second switch.
    if unsafe { DEEP_IN } { return f(); }
    let base = unsafe {
        if DEEP_STACK.is_null() { DEEP_STACK = alloc::vec![0u8; DEEP].leak().as_mut_ptr(); }
        DEEP_STACK
    };
    let mut f = Some(f);
    let mut out: Option<R> = None;
    let mut body = || out = Some((f.take().unwrap())());
    let mut call: &mut dyn FnMut() = &mut body;
    extern "C" fn enter(call: *mut &mut dyn FnMut()) { unsafe { (*call)() } }
    let top = (base as usize + DEEP) & !15;
    // SAFETY: the stack is ours alone (Files serves one request at a time), and r12 is
    // callee-saved, so it brings the old stack pointer back across the call.
    unsafe {
        DEEP_IN = true;
        core::arch::asm!("mov r12, rsp", "mov rsp, {top}", "call {enter}", "mov rsp, r12",
            top = in(reg) top, enter = sym enter, in("rdi") &mut call as *mut &mut dyn FnMut(),
            out("r12") _, clobber_abi("C"));
        DEEP_IN = false;
    }
    out.unwrap()
}
fn home() -> &'static mut Option<Fs<'static, HomeDev>> { unsafe { &mut *core::ptr::addr_of_mut!(HOME) } }

pub fn owns(path: &[u8]) -> bool { path == h::HOME || path.starts_with(b"/home/") }
pub fn state(main: &mut Fs<'static, FsDisk>) -> u32 {
    if home().is_some() { h::STATE_UNLOCKED } else if main.open(h::IMAGE).is_ok() { h::STATE_LOCKED } else { h::STATE_NONE }
}

/// One of SETUP, UNLOCK, LOCK, STATE. `secret` is the password, copied out of the region.
pub fn control(main: &mut Fs<'static, FsDisk>, kind: u8, mib: u64, secret: &[u8]) -> Result<u32, u8> {
    deep(|| control_here(main, kind, mib, secret))
}
fn control_here(main: &mut Fs<'static, FsDisk>, kind: u8, mib: u64, secret: &[u8]) -> Result<u32, u8> {
    match kind {
        h::STATE => Ok(state(main)),
        h::SETUP => setup(main, mib, secret),
        h::UNLOCK => unlock(main, secret),
        h::LOCK => lock(main),
        _ => Err(w::INVALID),
    }
}
/// The header, built on the heap: the key derivation runs deep in Files' small stack.
#[inline(never)]
fn wrap(password: &[u8], salt: &[u8; 32], master: &[u8; KEY]) -> Result<alloc::boxed::Box<[u8; HEADER]>, u8> {
    let mut h = alloc::boxed::Box::new([0u8; HEADER]);
    sex_crypt::header_into(&mut h, password, salt, master, Cost::SMALL).map_err(crypt_status)?;
    Ok(h)
}
#[inline(never)]
fn setup(main: &mut Fs<'static, FsDisk>, mib: u64, password: &[u8]) -> Result<u32, u8> {
    if main.open(h::IMAGE).is_ok() { return Err(w::EXISTS); }
    if password.len() < h::PASSWORD_MIN { return Err(w::NAME); }
    if !(16..=1 << 20).contains(&mib) { return Err(w::INVALID); }
    // Nothing may already be in `/home`: it would vanish under the volume.
    if let Ok(st) = main.open(h::HOME) {
        let mut name = [0u8; sexfs::NAME_MAX];
        if main.read_dir(st.id, 0, &mut name).map_err(crate::fs_service::status)?.is_some() { return Err(w::NOT_EMPTY); }
    }
    let mut seed = [0u8; 32 + KEY];
    random(&mut seed).ok_or(w::UNSUPPORTED)?;
    let salt: [u8; 32] = seed[..32].try_into().unwrap();
    let master: [u8; KEY] = seed[32..].try_into().unwrap();
    let start = sex_pdx::get_ticks();
    let header = wrap(password, &salt, &master)?;
    let blocks = mib * 1024 * 1024 / BLOCK as u64;
    let now = sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0);
    let made = (|| -> Result<u64, Error> {
        if main.open(h::HOME).is_err() { main.mkdir(h::HOME, now)?; }
        let id = main.create(h::IMAGE, now)?;
        main.write(id, 0, &header[..], now)?;
        main.truncate(id, HomeDev::at(blocks), now)?;
        main.commit()?;
        Ok(id)
    })();
    let id = match made { Ok(id) => id, Err(e) => { let _ = main.rollback(); return Err(crate::fs_service::status(e)); } };
    let dev = HomeDev { id, blocks, xts: Xts::new(&master) };
    let formatted = lend(main, || Fs::format(dev, memory(blocks)).and_then(|mut fs| { fs.commit()?; Ok(fs) }));
    match formatted {
        Ok(fs) => {
            *home() = Some(fs);
            sex_pdx::serial_record!("[files.home.setup] mib={} kdf_ms={} cost_kib={}", mib, sex_pdx::get_ticks().saturating_sub(start), Cost::SMALL.memory_kib);
            Ok(0)
        }
        Err(e) => { sex_pdx::serial_record!("[files.home.setup.failed] error={:?}", e); Err(crate::fs_service::status(e)) }
    }
}
#[inline(never)]
fn unlock(main: &mut Fs<'static, FsDisk>, password: &[u8]) -> Result<u32, u8> {
    if home().is_some() { return Ok(0); }
    let st = main.open(h::IMAGE).map_err(|_| w::NOT_FOUND)?;
    let mut header = vec![0u8; HEADER];
    let n = main.read(st.id, 0, &mut header).map_err(crate::fs_service::status)?;
    if n != HEADER { return Err(w::CORRUPT); }
    let start = sex_pdx::get_ticks();
    let master = match sex_crypt::unlock(&header, password) {
        Ok(k) => k,
        Err(e) => {
            sex_pdx::serial_record!("[files.home.unlock.refused] why={:?} ms={}", e, sex_pdx::get_ticks().saturating_sub(start));
            return Err(crypt_status(e));
        }
    };
    let blocks = (st.size / BLOCK as u64).saturating_sub(1);
    let dev = HomeDev { id: st.id, blocks, xts: Xts::new(&master) };
    match lend(main, || Fs::mount(dev, memory(blocks))) {
        Ok(fs) => {
            let s = fs.statfs();
            *home() = Some(fs);
            sex_pdx::serial_record!("[files.home.unlocked] ms={} total={} free={} generation={}", sex_pdx::get_ticks().saturating_sub(start), s.total, s.free, s.generation);
            Ok(0)
        }
        Err(e) => { sex_pdx::serial_record!("[files.home.mount.failed] error={:?}", e); Err(crate::fs_service::status(e)) }
    }
}
#[inline(never)]
fn lock(main: &mut Fs<'static, FsDisk>) -> Result<u32, u8> {
    let Some(mut fs) = home().take() else { return Ok(0) };
    let r = lend(main, || fs.commit());
    if let Err(e) = r { *home() = Some(fs); sex_pdx::serial_record!("[files.home.lock.failed] error={:?}", e); return Err(crate::fs_service::status(e)); }
    drop(fs);
    crate::fs_service::revoke_under(h::HOME);
    sex_pdx::serial_record!("[files.home.locked]");
    Ok(0)
}
fn crypt_status(e: sex_crypt::Error) -> u8 {
    match e { sex_crypt::Error::WrongPassword => h::WRONG_PASSWORD, sex_crypt::Error::NotEncrypted | sex_crypt::Error::Corrupt => w::CORRUPT, sex_crypt::Error::NoMemory => w::NO_SPACE }
}

/// A request on `/home` or under it; `inner` is the path inside the volume.
pub fn run(main: &mut Fs<'static, FsDisk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, u8> {
    deep(|| run_here(main, r, path, data))
}
fn run_here(main: &mut Fs<'static, FsDisk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, u8> {
    let strip = |p: &[u8]| -> alloc::vec::Vec<u8> { let i = &p[h::HOME.len().min(p.len())..]; if i.is_empty() { b"/".to_vec() } else { i.to_vec() } };
    // A rename's two halves both lose the prefix.
    let inner = match path.iter().position(|b| *b == 0) {
        Some(i) if r.kind == w::RENAME => { let mut v = strip(&path[..i]); v.push(0); v.extend_from_slice(&strip(&path[i + 1..])); v }
        _ => strip(path),
    };
    let inner = &inner[..];
    let Some(fs) = home().as_mut() else {
        // Locked: the folder is there and empty, and nothing in it can be reached.
        return match (r.kind, inner == b"/") {
            (w::STAT, true) => { data[..32].fill(0); data[0] = 1; Ok(32) }
            (w::LIST, true) => { data[..8].fill(0); Ok(w::LIST_HEAD as u32) }
            _ => Err(h::LOCKED),
        };
    };
    if matches!(r.kind, w::REMOVE | w::RENAME) && inner == b"/" { return Err(w::DENIED); }
    let read_only = matches!(r.kind, w::STAT | w::LIST | w::READ | w::STATFS);
    let result = lend(main, || crate::fs_service::run_on(fs, r, inner, data));
    match result {
        // Writes wait for home's next commit, which the system's batching drives.
        Ok(v) => Ok(v),
        Err(e) => {
            if !read_only && !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) { let _ = lend(main, || fs.rollback()); }
            Err(crate::fs_service::status(e))
        }
    }
}
/// A rename with one half in `/home` and the other outside: another disk.
pub fn crosses(a: &[u8], b: &[u8]) -> bool { owns(a) != owns(b) }
/// SYNC with no path: home is written out too.
pub fn sync(main: &mut Fs<'static, FsDisk>) {
    if home().is_some() { deep(|| sync_here(main)) }
}
fn sync_here(main: &mut Fs<'static, FsDisk>) {
    if let Some(fs) = home().as_mut() { if let Err(e) = lend(main, || fs.commit()) { sex_pdx::serial_record!("[files.home.sync.failed] error={:?}", e); } }
}
