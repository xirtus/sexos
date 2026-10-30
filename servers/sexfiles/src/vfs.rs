extern crate alloc;
use crate::backends::ramfs::RamFs;
use crate::backends::FsBackend;
use crate::backends::diskfs::{DiskFs, DiskManifestEntryV1, DISKFS_MANIFEST_OBJECT_PATH, DISKFS_MANIFEST_MAGIC, DISKFS_MANIFEST_FLAG_READ, DISKFS_MANIFEST_FLAG_WRITE, DISKFS_MANIFEST_LBA};
use sex_pdx::SLOT_BUF_LEND;
use crate::messages;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::RwLock;

/// VFS operation counters (diagnostic only).

// SERIAL_DIET_V1: shared budget for hot-loop diagnostics. Early boot keeps
// the full trace (first 200 lines); steady-state loops go quiet so
// serial VM-exits stop dominating wall clock. Error and gate-required
// markers are NOT routed through this.
static mut HOT_LOG_BUDGET: u32 = 200;
#[inline(always)]
fn hot_log() -> bool {
    unsafe {
        if HOT_LOG_BUDGET > 0 { HOT_LOG_BUDGET -= 1; true } else { false }
    }
}

pub static IPC_OPS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// The single RamFS instance backing all VFS operations.
pub fn retire_app_subject(target:u32)->(usize,usize) {
    DISKFS_SELECTED_PATH_ID[target as usize % DISKFS_CLIENT_SLOTS].store(0,Ordering::Relaxed);
    document_cancel(target);
    RAMFS.retire_subject(target)
}

pub static RAMFS: RamFs = RamFs::new();

// ── DiskFS bridge buffer state ──────────────────────────────────────────────
/// Cached view of the immutable boot-assigned block buffer (slot 17).
/// Resolution is idempotent and cannot allocate frames or change capabilities.
/// SexFiles owns this buffer for its lifetime; Linen never sees it.
static DISKFS_BRIDGE_BUF_VA: AtomicU64 = AtomicU64::new(0);

/// Whether the manifest at LBA 2046 has been validated (or bootstrapped).
/// Set to true after the first successful diskfs_ensure_manifest() call.
/// Avoids redundant NVMe manifest reads on every bridge WRITE/READ.
static DISKFS_MANIFEST_READY: AtomicU64 = AtomicU64::new(0);

/// Currently selected DiskFS object path_id for bridge operations,
/// PER CALLER PD. Default 0 = /disk/sexfiles-proof-v1.
/// SEXFILES_DEFER_V1 made concurrent clients real: a single global
/// selection let interleaved clients clobber each other and do I/O on
/// the wrong object. Indexed by caller_pd % DISKFS_CLIENT_SLOTS.
/// Set via OP_DISKFS_SELECT. Read by bridge WRITE/READ/STAT/HASH handlers.
const DISKFS_CLIENT_SLOTS: usize = 32;
static DISKFS_SELECTED_PATH_ID: [AtomicU64; DISKFS_CLIENT_SLOTS] =
    [const { AtomicU64::new(0) }; DISKFS_CLIENT_SLOTS];

fn diskfs_selected_for(caller_pd: u32) -> u64 {
    DISKFS_SELECTED_PATH_ID[caller_pd as usize % DISKFS_CLIENT_SLOTS].load(Ordering::Relaxed)
}

/// Whether a SELECT has been issued at least once since boot.
static DISKFS_SELECT_USED: AtomicU64 = AtomicU64::new(0);

static DISKFS_BRIDGE_REUSE_PRINTED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn diskfs_bridge_get_buf_va() -> u64 {
    let va = DISKFS_BRIDGE_BUF_VA.load(Ordering::Relaxed);
    if va != 0 && va != u64::MAX {
        if DISKFS_BRIDGE_REUSE_PRINTED.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.buf.reuse] va={:#x}",
                va
            );
        }
        return va;
    }
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    let Ok(info) = sex_pdx::memory_region_info(sex_pdx::SLOT_BLOCK_BUFFER)
        else { return u64::MAX; };
    if info.peer_pd.is_none() || info.length < 4096 { return u64::MAX; }
    let new_va = sex_pdx::memory_region_view(
        sex_pdx::SLOT_BLOCK_BUFFER, 0, 4096, QUERY_VIEW | READ | WRITE,
    ).map(|view| view.virtual_address).unwrap_or(u64::MAX);
    if new_va != 0 && new_va != u64::MAX {
        DISKFS_BRIDGE_BUF_VA.store(new_va, Ordering::Relaxed);
        sex_pdx::serial_record!(
            "[sexfiles.bridge.diskfs.buf.ready] buf_va={:#x} region_id={} length={} peer_pd={}",
            new_va, info.region_id, info.length, info.peer_pd.unwrap_or(0)
        );
    }
    new_va
}

// ── DiskFS bridge inline handlers ───────────────────────────────────────────

fn handle_diskfs_select(path_id: u64, caller_pd: u32) -> u64 {
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x3E select path_id={}",
        path_id
    ); }

    let buf_va = diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.select.err] reason=view_failed"
        );
        return messages::ERR_NOT_FOUND as u64;
    }
    if let Err(e) = v4_ensure(buf_va) {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.select.err] reason=v4_ensure_failed code={}", e);
        return e;
    }
    match v4_get(path_id) {
        Ok(_entry) => {
            DISKFS_SELECTED_PATH_ID[caller_pd as usize % DISKFS_CLIENT_SLOTS]
                .store(path_id, Ordering::Relaxed);
            if DISKFS_SELECT_USED.load(Ordering::Relaxed) == 0 {
                DISKFS_SELECT_USED.store(1, Ordering::Relaxed);
                crate::pdx::serial_println!(
                    "[sexfiles.bridge.diskfs.select.v1_single_client]"
                );
            }
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.select.ok] caller={} path_id={}", caller_pd, path_id
            );
            0
        }
        Err(e) => {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.select.err] path_id={} code={}",
                path_id, e
            );
            e
        }
    }
}

// DISKFS_V3: path-string resolution retired — selection resolves through
// the dynamic manifest table (v4_get).

fn handle_diskfs_write(byte_offset: u64, data_lo: u64, data_hi: u64, caller_pd: u32) -> u64 {
    if document_locked(diskfs_selected_for(caller_pd)) { return messages::ERR_FULL as u64; }
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x38 offset={}",
        byte_offset
    ); }

    // DISKFS_V4: explicit, enforced, visible cap — growth past this is
    // ERR_OVERFLOW, never silent truncation. byte_offset is caller-supplied
    // (an untrusted PDX message argument) — saturating_add so a malicious
    // or buggy near-u64::MAX offset can't wrap into a small, falsely-valid
    // value in a release build (no overflow-checks there).
    if byte_offset >= DISKFS_V4_MAX_OBJECT_BYTES
        || byte_offset.saturating_add(messages::DISKFS_MAX_WRITE as u64) > DISKFS_V4_MAX_OBJECT_BYTES
    {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=cap_exceeded offset={} cap={}",
            byte_offset, DISKFS_V4_MAX_OBJECT_BYTES
        );
        return messages::ERR_OVERFLOW as u64;
    }
    // A 16-byte chunk must not straddle a 4096-byte block. Real callers
    // always write 16-byte-aligned chunks from a 4096-aligned object base,
    // so this never legitimately triggers — it's a defensive bound.
    if (byte_offset % DISKFS_V4_BLOCK_BYTES).saturating_add(messages::DISKFS_MAX_WRITE as u64) > DISKFS_V4_BLOCK_BYTES {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=block_straddle offset={}",
            byte_offset
        );
        return messages::ERR_OVERFLOW as u64;
    }

    let buf_va = diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=view_failed buf_va={:#x}",
            buf_va
        );
        return messages::ERR_NOT_FOUND as u64;
    }

    if let Err(e) = v4_ensure(buf_va) {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=v4_ensure_failed code={}", e);
        return e;
    }
    let path_id = diskfs_selected_for(caller_pd);
    if path_id >= DISKFS_SLOTS as u64 {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=bad_path_id caller={} path_id={}",
            caller_pd, path_id);
        return messages::ERR_BAD_CMD as u64;
    }
    let i = path_id as usize;
    if !v4_in_use(i) {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.write.err] reason=slot_not_in_use caller={} path_id={}",
            caller_pd, path_id);
        return messages::ERR_NOT_FOUND as u64;
    }

    let mut entry = unsafe { V4_TABLE[i] };
    // Provably <= DISKFS_V4_MAX_OBJECT_BYTES by the cap_exceeded check
    // above; saturating regardless so that guarantee doesn't depend on
    // this line staying textually after it.
    let need_end = byte_offset.saturating_add(messages::DISKFS_MAX_WRITE as u64);
    // A never-written (empty) object has no indirect sector yet — that
    // reads back as "unreadable" (no SFEX magic), which is indistinguishable
    // here from corruption. That's fine: a genuinely non-empty object's
    // indirect sector was already validated by v4_bitmap_rebuild() at
    // mount time, so reaching here with size_bytes>0 and an unreadable
    // descriptor cannot happen in normal operation.
    // Cached: a save loop calls WRITE ~len/16 times against the same
    // selected object — re-reading the indirect sector from disk on every
    // one of them (V3 had no such read at all) was slow enough to blow
    // through the gate scripts' keystroke timing windows.
    let (mut extents, orig_count) = v4_cache_get(i, buf_va);
    let mut count = orig_count;
    let cur_capacity: u64 = (0..count).map(|k| extents[k].sector_count as u64 * 512).sum();

    if need_end > cur_capacity {
        let more_bytes = need_end - cur_capacity;
        let more_blocks = (more_bytes + DISKFS_V4_BLOCK_BYTES - 1) / DISKFS_V4_BLOCK_BYTES;
        let new_count = {
            let mut bm = V4_BITMAP.write();
            match v4_allocate(&mut bm, more_blocks, &mut extents, count) {
                Ok(n) => n,
                Err(e) => {
                    crate::pdx::serial_println!(
                        "[sexfiles.bridge.diskfs.write.err] reason=alloc_failed need_blocks={}",
                        more_blocks
                    );
                    return e as u64;
                }
            }
        };
        // Zero-fill every newly allocated block before anything references
        // it — never leak a previous tenant's content into fresh storage.
        for k in count..new_count {
            let e = extents[k];
            let mut s = 0u64;
            while s < e.sector_count as u64 {
                if let Err(er) = v4_zero_block(e.start_lba as u64 + s, buf_va) {
                    crate::pdx::serial_println!(
                        "[sexfiles.bridge.diskfs.write.err] reason=zero_fill_failed lba={} code={}",
                        e.start_lba as u64 + s, er);
                    let mut bm = V4_BITMAP.write();
                    v4_free_pool_only(&mut bm, &extents[count..new_count]);
                    return er;
                }
                s += DISKFS_V4_BLOCK_SECTORS;
            }
        }
        count = new_count;
    }

    let (piece, rel_off) = match v4_locate(&extents, count, byte_offset) {
        Ok(v) => v,
        Err(e) => {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.write.err] reason=locate_failed offset={} count={} code={}",
                byte_offset, count, e
            );
            if count > orig_count {
                let mut bm = V4_BITMAP.write();
                v4_free_pool_only(&mut bm, &extents[orig_count..count]);
            }
            return e;
        }
    };

    // Pack 16 bytes inline from data_lo + data_hi.
    let mut inline_data = [0u8; 16];
    {
        let lo = data_lo.to_le_bytes();
        let hi = data_hi.to_le_bytes();
        let mut i = 0;
        while i < 8 {
            inline_data[i] = lo[i];
            inline_data[i + 8] = hi[i];
            i += 1;
        }
    }

    let written = match DiskFs::diskfs_write_object_entry(piece, rel_off, &inline_data, buf_va) {
        Ok(n) => n,
        Err(e) => {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.write.err] offset={} code={}",
                byte_offset, e
            );
            if count > orig_count {
                let mut bm = V4_BITMAP.write();
                v4_free_pool_only(&mut bm, &extents[orig_count..count]);
            }
            return e;
        }
    };

    // Crash-safety ordering: commit the (possibly grown) indirect
    // descriptor before the manifest size that makes it visible.
    if count > orig_count {
        if DiskFs::diskfs_block_sync()!=0{return messages::ERR_BAD_CMD as u64;}
        if let Err(e) = v4_indirect_write(i, buf_va, &extents, count) {
            let mut bm = V4_BITMAP.write();
            v4_free_pool_only(&mut bm, &extents[orig_count..count]);
            return e;
        }
        if DiskFs::diskfs_block_sync()!=0{return messages::ERR_BAD_CMD as u64;}
        v4_cache_put(i, extents, count);
        // Deterministic injection point for crash-safety testing (see
        // scripts/diskfs_v4_crash_injection_gate.sh): the new extent is on
        // disk and readable by a rebuilt bitmap, but the manifest has not
        // yet published this object's new size. A crash landing exactly
        // here must still resolve to the OLD version on reboot.
        crate::pdx::serial_println!(
            "[sexfiles.diskfs.v4.crash_point.extent_committed] slot={} new_count={}",
            i, count
        );
    }
    if need_end > entry.size_bytes as u64 {
        entry.size_bytes = need_end as u32;
        unsafe { V4_TABLE[i] = entry; }
        if let Err(e) = v4_persist(buf_va) { return e; }
        if DiskFs::diskfs_block_sync()!=0{return messages::ERR_BAD_CMD as u64;}
        // Deterministic injection point: the manifest now publishes the new
        // size. A crash landing exactly here must resolve to the complete
        // NEW version on reboot, never a partial one.
        crate::pdx::serial_println!(
            "[sexfiles.diskfs.v4.crash_point.manifest_committed] slot={} size={}",
            i, entry.size_bytes
        );
    }

    crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.write.ok] offset={} written={}",
        byte_offset, written
    );
    written
}

fn handle_diskfs_read(byte_offset: u64, max_len: u64, caller_pd: u32) -> u64 {
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x39 offset={}",
        byte_offset
    ); }

    if max_len == 0 || max_len > messages::DISKFS_MAX_READ as u64 {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read.err] reason=bad_max_len max_len={}",
            max_len
        );
        return messages::ERR_OVERFLOW as u64;
    }

    let buf_va = diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read.err] reason=view_failed buf_va={:#x}",
            buf_va
        );
        return messages::ERR_NOT_FOUND as u64;
    }

    if let Err(e) = v4_ensure(buf_va) {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read.err] reason=v4_ensure_failed code={}", e);
        return e;
    }
    let path_id = diskfs_selected_for(caller_pd);
    let entry = match v4_get(path_id) {
        Ok(en) => en,
        Err(e) => return e,
    };

    // DISKFS_V4: bounded by the object's ACTUAL (variable) length — a read
    // at or past it is a deterministic error, never stale trailing bytes.
    // byte_offset/max_len are caller-supplied; saturating_add so a
    // near-u64::MAX offset can't wrap past this check in a release build.
    if byte_offset >= entry.size_bytes as u64 || byte_offset.saturating_add(max_len) > entry.size_bytes as u64 {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read.err] reason=read_past_end offset={} max_len={} size={}",
            byte_offset, max_len, entry.size_bytes
        );
        return messages::ERR_OVERFLOW as u64;
    }

    let (piece, rel_off) = match v4_resolve_offset(path_id as usize, byte_offset, buf_va) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let rlen = max_len as usize;
    let mut rbuf = [0u8; 8];
    match DiskFs::diskfs_read_object_entry(
        piece,
        rel_off,
        &mut rbuf[..rlen],
        buf_va,
    ) {
        Ok(n) => {
            // Pack up to 8 bytes into reply u64 (LE).
            let mut reply: u64 = 0;
            let mut i = 0;
            while i < n as usize && i < 8 {
                reply |= (rbuf[i] as u64) << (i * 8);
                i += 1;
            }
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.read.ok] offset={} read={}",
                byte_offset, n
            );
            if hot_log() { crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.read.reply] caller={} value={:#x} off={} read={}",
                caller_pd, reply, byte_offset, n
            ); }
            reply
        }
        Err(e) => {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.read.err] caller={} err={} off={}",
                caller_pd, e, byte_offset
            );
            e
        }
    }
}

/// OP_DISKFS_READ_V2 (0x4A): see messages.rs doc comment for the reply bit
/// layout. status byte in bits 63..56 is never shared with payload bits
/// (47..0), so a data byte >= 0x80 can never be misread as an error the
/// way OP_DISKFS_READ's full-width reply could.
fn handle_diskfs_read_v2(byte_offset: u64, want_len: u64, caller_pd: u32) -> u64 {
    // Every ERR_* constant is a small negative i64; callers here pass it
    // through as u64 (the huge-unsigned bit pattern), so recover its
    // magnitude before handing it to the shared encoder.
    let pack_err = |e: u64| -> u64 { sex_pdx::diskfs_v2_encode_err((e as i64).unsigned_abs()) };

    if want_len == 0 || want_len > messages::DISKFS_V2_MAX_READ as u64 {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read_v2.err] reason=bad_want_len want_len={}",
            want_len
        );
        return pack_err(messages::ERR_OVERFLOW as u64);
    }

    let buf_va = diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX {
        return pack_err(messages::ERR_NOT_FOUND as u64);
    }
    if let Err(e) = v4_ensure(buf_va) {
        return pack_err(e);
    }
    let path_id = diskfs_selected_for(caller_pd);
    let entry = match v4_get(path_id) {
        Ok(en) => en,
        Err(e) => return pack_err(e),
    };

    if byte_offset == entry.size_bytes as u64 {
        // Explicit EOF, not an error: offset sits exactly at the end.
        return sex_pdx::diskfs_v2_encode_eof();
    }
    if byte_offset > entry.size_bytes as u64
        || byte_offset.saturating_add(want_len) > entry.size_bytes as u64
    {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.read_v2.err] reason=read_past_end offset={} want_len={} size={}",
            byte_offset, want_len, entry.size_bytes
        );
        return pack_err(messages::ERR_OVERFLOW as u64);
    }

    let (piece, rel_off) = match v4_resolve_offset(path_id as usize, byte_offset, buf_va) {
        Ok(v) => v,
        Err(e) => return pack_err(e),
    };

    let rlen = want_len as usize;
    let mut rbuf = [0u8; messages::DISKFS_V2_MAX_READ];
    match DiskFs::diskfs_read_object_entry(piece, rel_off, &mut rbuf[..rlen], buf_va) {
        Ok(n) => {
            let n = n as usize;
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.read_v2.ok] offset={} read={}",
                byte_offset, n
            );
            sex_pdx::diskfs_v2_encode_ok(n, &rbuf[..n])
        }
        Err(e) => {
            crate::pdx::serial_println!(
                "[sexfiles.bridge.diskfs.read_v2.err] caller={} err={} off={}",
                caller_pd, e, byte_offset
            );
            pack_err(e)
        }
    }
}

fn handle_diskfs_truncate(new_len: u64, caller_pd: u32) -> u64 {
    if document_locked(diskfs_selected_for(caller_pd)) { return messages::ERR_FULL as u64; }
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x49 new_len={}",
        new_len
    ); }
    let buf_va = diskfs_bridge_get_buf_va();
    if buf_va == 0 || buf_va == u64::MAX {
        return messages::ERR_NOT_FOUND as u64;
    }
    if let Err(e) = v4_ensure(buf_va) { return e; }
    let path_id = diskfs_selected_for(caller_pd);
    if path_id >= DISKFS_SLOTS as u64 { return messages::ERR_BAD_CMD as u64; }
    let i = path_id as usize;
    if !v4_in_use(i) { return messages::ERR_NOT_FOUND as u64; }

    let mut entry = unsafe { V4_TABLE[i] };
    if new_len > entry.size_bytes as u64 {
        // Shrink/no-op only — growth happens implicitly via WRITE.
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.truncate.err] reason=would_grow new_len={} size={}",
            new_len, entry.size_bytes
        );
        return messages::ERR_OVERFLOW as u64;
    }
    if new_len == entry.size_bytes as u64 {
        return new_len;
    }

    // Shrink-commit-metadata-first ordering (see module doc): the object
    // is visibly short as of this write, before any extent bookkeeping.
    entry.size_bytes = new_len as u32;
    unsafe { V4_TABLE[i] = entry; }
    if let Err(e) = v4_persist(buf_va) { return e; }
    // Deterministic injection point (see scripts/diskfs_v4_crash_injection_gate.sh):
    // this is the atomicity boundary a WRITE-loop-then-TRUNCATE save (spindle's
    // filldoc, quil's persist_save) actually relies on for "the complete new
    // version is durable" -- not the last WRITE, which may still be padded to
    // a block/chunk boundary the caller intends to trim away.
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.crash_point.manifest_committed] slot={} size={}",
        i, entry.size_bytes
    );

    let (mut extents, count) = v4_cache_get(i, buf_va);
    let mut cum = 0u64;
    let mut keep = 0usize;
    let mut freed: [V4Extent; DISKFS_V4_MAX_EXTENTS] = [V4Extent::default(); DISKFS_V4_MAX_EXTENTS];
    let mut freed_n = 0usize;
    for k in 0..count {
        let e = extents[k];
        let cap = e.sector_count as u64 * 512;
        if cum + cap <= new_len {
            extents[keep] = e; keep += 1;
        } else if cum >= new_len {
            freed[freed_n] = e; freed_n += 1;
        } else {
            // Straddles the new boundary: keep the whole extent (block
            // granularity — freeing a partial block isn't worth the extra
            // bookkeeping at this bar) but zero its tail so a later
            // regrow never exposes the just-shrunk content.
            extents[keep] = e; keep += 1;
            if let Err(er) = v4_zero_tail(e, new_len - cum, buf_va) { return er; }
        }
        cum += cap;
    }
    if freed_n > 0 {
        if let Err(e) = v4_indirect_write(i, buf_va, &extents, keep) { return e; }
        v4_cache_put(i, extents, keep);
        let mut bm = V4_BITMAP.write();
        v4_free_pool_only(&mut bm, &freed[..freed_n]);
    }
    crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.truncate.ok] new_len={} freed_extents={}",
        new_len, freed_n
    );
    new_len
}

fn handle_diskfs_flush() -> u64 {
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x3A"
    ); }
    let status = DiskFs::diskfs_fsync();
    if status == 0 {
        crate::pdx::serial_println!("[sexfiles.bridge.diskfs.flush.ok]");
    } else {
        crate::pdx::serial_println!(
            "[sexfiles.bridge.diskfs.flush.err] status={} honest=flush_not_emulated_by_qemu_nvme",
            status
        );
    }
    status
}

fn handle_diskfs_stat(caller_pd: u32) -> u64 {
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x3B"
    ); }
    let path_id = diskfs_selected_for(caller_pd);
    let entry = match v4_get(path_id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let name = entry.name;
    let path = &name[..v4_name_len(&name)];
    // DISKFS_V4: size is the object's exact current length, not a constant.
    let flags: u64 = (DISKFS_MANIFEST_FLAG_READ | DISKFS_MANIFEST_FLAG_WRITE) as u64;
    let size: u64 = entry.size_bytes as u64;
    let packed = (flags << 32) | (size & 0xFFFF_FFFF);
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.stat.ok] path_id={} path={} size={} flags={:#x}",
        path_id,
        core::str::from_utf8(path).unwrap_or("?"),
        size, flags
    ); }
    packed
}

fn handle_diskfs_manifest_hash(caller_pd: u32) -> u64 {
    if hot_log() { crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.recv] op=0x3C"
    ); }
    let path_id = diskfs_selected_for(caller_pd);
    let entry = match v4_get(path_id) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let name = entry.name;
    let hash = DiskFs::proof_manifest_name_hash(&name[..v4_name_len(&name)]);
    crate::pdx::serial_println!(
        "[sexfiles.bridge.diskfs.manifest_hash.ok] path_id={} hash={:#x}",
        path_id, hash
    );
    hash
}

/// Route a PDX message to the appropriate backend handler.
/// Called from the trampoline message loop.
/// `caller_pd` is the PD of the requesting process (from PDX message).

// ═══════════════════════════════════════════════════════════════════════════
//  DISKFS_V4 — variable-length dynamic object store (supersedes V3's fixed
//  4096-byte-per-object limit).
//
//  Manifest — ONE 512-byte sector at LBA 2046 (unchanged location):
//    [0..8)   magic  "SDISKMV1" (unchanged — version field disambiguates)
//    [8..10)  version = 4
//    [10..12) entry_count = 15
//    [12..16) generation (u32, bumped on every metadata change)
//    16 + i*32 .. : entry i (15 entries × 32 bytes = 480):
//       name[24]       zero-padded ASCII; name[0]==0 → slot free
//       +24 size_bytes u32  exact logical content length (0 = empty)
//       +28 checksum   u16  over (name || size_bytes || gen), corruption guard
//       +30 gen        u16  bumped on create/delete/rename; stale-id detection
//
//  Indirect extent descriptor — one 512-byte sector PER SLOT, at a fixed,
//  deterministic LBA (DISKFS_V4_INDIRECT_BASE_LBA - slot_index), so no
//  separate allocator is needed just to place it:
//    [0..4)  magic "SFEX"
//    [4..6)  version = 1
//    [6..8)  extent_count (0..DISKFS_V4_MAX_EXTENTS)
//    [8..12) checksum (FNV-1a over the live extent bytes)
//    12+k*4..: extent k: start_lba u16 (raw sector number), sector_count u16
//  Extents store raw LBA/sector-count rather than pool-relative block
//  indices so a migrated legacy object (fixed LBA, not block-aligned to
//  the pool's origin) fits the same representation as a freshly allocated
//  one — no special-case field needed.
//  A slot's indirect sector is meaningful only while its manifest entry is
//  in use; create/delete always rewrite it fully, so a stale descriptor
//  left by an interrupted operation is never trusted on its own — the
//  manifest entry is the sole authority for "does this object exist".
//
//  Content pool — DISKFS_V4_POOL_BLOCKS blocks of 4096 bytes each, at
//  LBA [0, POOL_BLOCKS*8), well clear of the indirect region (starts at
//  LBA 1911) and the legacy V3 slot / V4 manifest region (LBA 1926..2047).
//  Free/used state is NOT persisted as its own structure — nothing to
//  corrupt independently: it is rebuilt every mount by scanning the
//  indirect descriptor of every in-use manifest slot and marking its
//  pool-region extents allocated. A block claimed by two slots at once
//  marks the later slot corrupt and drops it rather than trusting it —
//  the same "drop, don't trust" policy V3 used for bad slot geometry.
//
//  Crash-safety ordering (see diskfs_recovery_gate):
//    grow:   allocate (in-memory bitmap only) -> zero-fill new block(s) on
//            disk -> write the requested content -> commit indirect
//            descriptor -> commit manifest size_bytes.
//            A crash before the indirect commit leaves new blocks written
//            but unreferenced (invisible; reclaimed as free next mount).
//            A crash before the manifest commit leaves the object at its
//            old (smaller) size — old valid version, nothing exposed.
//    shrink (TRUNCATE): commit manifest size_bytes DOWN first, then drop
//            the now-unused trailing extents from the indirect descriptor.
//            A crash between the two leaves a few blocks "reserved but
//            unreachable" (reclaimed once the indirect update completes);
//            content past the committed size_bytes is bounds-checked out
//            of every read either way, so no stale trailing bytes are
//            ever visible regardless of which side of the crash it lands.
//    delete: clear the manifest name (object disappears from every
//            listing/read/write immediately) -> free its extents from the
//            live bitmap -> zero the indirect descriptor. A crash between
//            steps still hides the object (name already cleared); its
//            blocks are reclaimed on the next mount's rebuild regardless.
//
//  Migration from V3: the 3 legacy system objects (sexfiles-proof-v1,
//  linen-object-v1, quil-object-v1) already live at fixed, contiguous
//  LBAs outside the new content pool. V3->V4 upgrade wraps each as a
//  single-extent V4 entry pointing at its EXISTING physical location —
//  no data is copied or moved, so their content survives byte-for-byte.
//  Their reported size_bytes becomes 4096, matching what V3 always
//  exposed for them.
// ═══════════════════════════════════════════════════════════════════════════
pub const DISKFS_SLOTS: usize = 15;
pub const DISKFS_V3_NAME_MAX: usize = 24;
const DISKFS_V3_VERSION: u16 = 3; // still recognized, for migration only
pub const DISKFS_V4_VERSION: u16 = 4;
/// System objects (slots 0-2) cannot be deleted or renamed.
const DISKFS_V3_SYSTEM_SLOTS: u64 = 3;

const DISKFS_V4_BLOCK_SECTORS: u64 = sex_pdx::DISKFS_V4_BLOCK_SECTORS;
const DISKFS_V4_BLOCK_BYTES: u64 = 4096;
/// sexdrive's real NVMe write path (apps/sexdrive write_guard_allows) only
/// accepts writes into pre-declared safe ranges — a deliberate bring-up
/// guard against writing unknown disk regions, not a bug. LBA [0,47] and
/// [128,2019] are already allowlisted for SexFS v0; LBA [48,127] is NOT.
/// The content pool sits inside the [128,2019] range specifically to land
/// entirely on allowed ground without needing a guard change.
///
/// The pool's exact placement is NOT chosen locally anymore — it comes
/// from crate::sex_pdx's canonical disk-layout module, the single source
/// of truth every fixed-LBA region in the system must be declared in and
/// checked against at compile time (`const _: () = assert!(...)` in
/// sex-pdx). This exists because the pool originally started at LBA 128,
/// picked independently of apps/sexdrive's own boot-time self-test
/// (nvme_multiblock_write_readback_proof), which ALSO unconditionally
/// claims LBA 128..131 on every boot with no gate — no code shared
/// between the two crates meant nothing caught the collision until a
/// reboot silently ate real DiskFS content. See
/// docs/handoff/SEXDRIVE_NVME_QUEUE_WRAP_V1.md for the full incident
/// (initially misdiagnosed as an NVMe queue-wrap bug before the actual
/// LBA collision was found).
const DISKFS_V4_POOL_BASE_LBA: u64 = sex_pdx::DISKFS_V4_POOL_BASE_LBA;
/// Content pool: blocks 0..POOL_BLOCKS at LBA [POOL_BASE_LBA, POOL_BASE_LBA
/// + POOL_BLOCKS*8) — comfortably inside the allowed [128,2019] range,
/// clear of every sexdrive self-test region (including the ones gated off
/// in normal builds — see sex-pdx's compile-time asserts), and well clear
/// of the indirect-descriptor region.
const DISKFS_V4_POOL_BLOCKS: u64 = sex_pdx::DISKFS_V4_POOL_BLOCKS;
/// Per-object cap: 16 blocks = 64 KiB. Explicit, enforced (ERR_OVERFLOW),
/// visible — never silently truncated.
pub const DISKFS_V4_MAX_OBJECT_BLOCKS: u32 = 16;
pub const DISKFS_V4_MAX_OBJECT_BYTES: u64 = DISKFS_V4_MAX_OBJECT_BLOCKS as u64 * DISKFS_V4_BLOCK_BYTES;
/// Bounded fragmentation: an object's content may live in up to this many
/// discontiguous extents before growth is refused (ERR_FULL).
const DISKFS_V4_MAX_EXTENTS: usize = 8;
/// Indirect descriptor for slot i lives at this LBA minus i (15 sectors,
/// the canonical range from sex_pdx::DISKFS_V4_INDIRECT_BASE_LBA up to
/// this value inclusive), just below the legacy V3 slot region.
const DISKFS_V4_INDIRECT_BASE_LBA: u64 =
    sex_pdx::DISKFS_V4_INDIRECT_BASE_LBA + sex_pdx::DISKFS_V4_INDIRECT_SECTORS - 1;
const DISKFS_V4_EXTENT_MAGIC: [u8; 4] = *b"SFEX";

fn v4_indirect_lba(i: usize) -> u64 { DISKFS_V4_INDIRECT_BASE_LBA - i as u64 }

/// One object's content extent: `sector_count` sectors starting at raw
/// LBA `start_lba`.
#[derive(Clone, Copy, Default)]
struct V4Extent { start_lba: u16, sector_count: u16 }

#[derive(Clone, Copy)]
struct V4Entry {
    name: [u8; DISKFS_V3_NAME_MAX],
    size_bytes: u32,
    gen: u16,
}
const V4_EMPTY: V4Entry = V4Entry { name: [0u8; DISKFS_V3_NAME_MAX], size_bytes: 0, gen: 0 };

static mut V4_TABLE: [V4Entry; DISKFS_SLOTS] = [V4_EMPTY; DISKFS_SLOTS];
static V4_LOADED: AtomicU64 = AtomicU64::new(0);
static V4_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Derived (not persisted) free/used map over the content pool — see
/// module doc. 176 bits fits in 3 u64 words.
static V4_BITMAP: RwLock<[u64; 3]> = RwLock::new([0u64; 3]);

fn v4_name_len(name: &[u8; DISKFS_V3_NAME_MAX]) -> usize {
    let mut n = 0;
    while n < DISKFS_V3_NAME_MAX && name[n] != 0 { n += 1; }
    n
}

fn v4_in_use(i: usize) -> bool {
    unsafe { V4_TABLE[i].name[0] != 0 }
}

fn v4_checksum_entry(e: &V4Entry) -> u16 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in e.name.iter() { h ^= b as u32; h = h.wrapping_mul(0x0100_0193); }
    for b in e.size_bytes.to_le_bytes() { h ^= b as u32; h = h.wrapping_mul(0x0100_0193); }
    for b in e.gen.to_le_bytes() { h ^= b as u32; h = h.wrapping_mul(0x0100_0193); }
    (h ^ (h >> 16)) as u16
}

fn v4_bitmap_test(bm: &[u64; 3], block: u64) -> bool {
    let w = (block / 64) as usize; let b = block % 64;
    bm[w] & (1u64 << b) != 0
}
fn v4_bitmap_set(bm: &mut [u64; 3], block: u64) {
    let w = (block / 64) as usize; let b = block % 64;
    bm[w] |= 1u64 << b;
}
fn v4_bitmap_clear(bm: &mut [u64; 3], block: u64) {
    let w = (block / 64) as usize; let b = block % 64;
    bm[w] &= !(1u64 << b);
}

/// Single-slot extent cache. A save loop issues many small WRITE (or READ)
/// calls in a row against the SAME selected object — without this, every
/// one of them re-reads the 512-byte indirect descriptor from disk before
/// touching content, tripling round-trip count on what used to be a single
/// R-M-W per call under V3 (which had no indirection at all). Keyed by
/// slot only (not per-caller): concurrent access to a different object
/// just misses and refills, never returns wrong data — correctness comes
/// from always refilling/updating on any mutation, not from the cache
/// being "sticky" to one caller.
#[derive(Clone, Copy)]
struct V4Cache { slot: usize, extents: [V4Extent; DISKFS_V4_MAX_EXTENTS], count: usize }
static V4_EXTENT_CACHE: RwLock<Option<V4Cache>> = RwLock::new(None);

fn v4_cache_put(i: usize, extents: [V4Extent; DISKFS_V4_MAX_EXTENTS], count: usize) {
    *V4_EXTENT_CACHE.write() = Some(V4Cache { slot: i, extents, count });
}

fn v4_cache_invalidate(i: usize) {
    let mut c = V4_EXTENT_CACHE.write();
    if matches!(*c, Some(cc) if cc.slot == i) { *c = None; }
}

/// Cached read of a slot's extents: hit the in-memory cache if it's for
/// this slot, otherwise fall through to disk and refill.
fn v4_cache_get(i: usize, buf_va: u64) -> ([V4Extent; DISKFS_V4_MAX_EXTENTS], usize) {
    if let Some(cc) = *V4_EXTENT_CACHE.read() {
        if cc.slot == i { return (cc.extents, cc.count); }
    }
    let (extents, count) = v4_indirect_read(i, buf_va).unwrap_or(([V4Extent::default(); DISKFS_V4_MAX_EXTENTS], 0));
    v4_cache_put(i, extents, count);
    (extents, count)
}

/// Read a slot's indirect extent descriptor. Err on missing magic, bad
/// version, or checksum mismatch — callers treat that identically to "no
/// extents yet" (see handle_diskfs_write's comment on why that's safe).
fn v4_indirect_read(i: usize, buf_va: u64) -> Result<([V4Extent; DISKFS_V4_MAX_EXTENTS], usize), ()> {
    let mut sector = [0u8; 512];
    if DiskFs::diskfs_block_read(v4_indirect_lba(i) * 512, 512, SLOT_BUF_LEND) != 0 { return Err(()); }
    unsafe {
        let p = buf_va as *const u8;
        for k in 0..512 { sector[k] = core::ptr::read_volatile(p.add(k)); }
    }
    for k in 0..4 { if sector[k] != DISKFS_V4_EXTENT_MAGIC[k] { return Err(()); } }
    let version = u16::from_le_bytes(sector[4..6].try_into().unwrap());
    if version != 1 { return Err(()); }
    let count = u16::from_le_bytes(sector[6..8].try_into().unwrap()) as usize;
    if count > DISKFS_V4_MAX_EXTENTS { return Err(()); }
    let stored_cs = u32::from_le_bytes(sector[8..12].try_into().unwrap());
    let mut extents = [V4Extent::default(); DISKFS_V4_MAX_EXTENTS];
    let mut h: u32 = 0x811c_9dc5;
    for k in 0..count {
        let off = 12 + k * 4;
        let sb = u16::from_le_bytes(sector[off..off + 2].try_into().unwrap());
        let sc = u16::from_le_bytes(sector[off + 2..off + 4].try_into().unwrap());
        if sex_pdx::ranges_overlap(sb as u64,sc as u64,sex_pdx::SPINDLE_HISTORY_BASE_LBA,sex_pdx::SPINDLE_HISTORY_SECTORS){return Err(())}
        extents[k] = V4Extent { start_lba: sb, sector_count: sc };
        for b in sector[off..off + 4].iter() { h ^= *b as u32; h = h.wrapping_mul(0x0100_0193); }
    }
    if h != stored_cs { return Err(()); }
    Ok((extents, count))
}

/// Write (and read-back verify) a slot's indirect extent descriptor.
fn v4_indirect_write(i: usize, buf_va: u64, extents: &[V4Extent], count: usize) -> Result<(), u64> {
    let mut sector = [0u8; 512];
    sector[0..4].copy_from_slice(&DISKFS_V4_EXTENT_MAGIC);
    sector[4..6].copy_from_slice(&1u16.to_le_bytes());
    sector[6..8].copy_from_slice(&(count as u16).to_le_bytes());
    let mut h: u32 = 0x811c_9dc5;
    for k in 0..count {
        let off = 12 + k * 4;
        sector[off..off + 2].copy_from_slice(&extents[k].start_lba.to_le_bytes());
        sector[off + 2..off + 4].copy_from_slice(&extents[k].sector_count.to_le_bytes());
        for b in sector[off..off + 4].iter() { h ^= *b as u32; h = h.wrapping_mul(0x0100_0193); }
    }
    sector[8..12].copy_from_slice(&h.to_le_bytes());
    unsafe {
        let p = buf_va as *mut u8;
        for k in 0..512 { core::ptr::write_volatile(p.add(k), sector[k]); }
    }
    let lba = v4_indirect_lba(i);
    let status = DiskFs::diskfs_block_write(lba * 512, 512, SLOT_BUF_LEND);
    if status != 0 { return Err(status); }
    let mut check = [0u8; 512];
    if DiskFs::diskfs_block_read(lba * 512, 512, SLOT_BUF_LEND) != 0 {
        return Err(messages::ERR_BAD_CMD as u64);
    }
    unsafe {
        let p = buf_va as *const u8;
        for k in 0..512 { check[k] = core::ptr::read_volatile(p.add(k)); }
    }
    if check != sector { return Err(messages::ERR_BAD_CMD as u64); }
    Ok(())
}

/// Zero-fill one freshly reserved 4KiB pool block before publishing it.
/// The existing peer-bound block region and permanent DMA lease cover one
/// full page. The driver validates the entire interval inside the V4 pool.
fn v4_zero_block(start_lba: u64, buf_va: u64) -> Result<(), u64> {
    unsafe {
        let p = buf_va as *mut u8;
        for k in 0..4096 { core::ptr::write_volatile(p.add(k), 0u8); }
    }
    let status=DiskFs::diskfs_block_write(start_lba*512,4096,SLOT_BUF_LEND);
    if status==0 {Ok(())}else{Err(status)}
}

/// Zero bytes [keep_len, capacity) of one extent, at 512-byte sector
/// granularity — one disk round-trip per sector instead of one per 16-byte
/// inline-message chunk (32x fewer round-trips). The sector straddling
/// `keep_len` needs a read-modify-write to preserve the bytes before it;
/// every sector after that is fully zeroed and written directly, same as
/// `v4_zero_block`. Bounded by DISKFS_V4_MAX_OBJECT_BYTES per extent, so
/// this loop is small.
fn v4_zero_tail(e: V4Extent, keep_len: u64, buf_va: u64) -> Result<(), u64> {
    let cap = e.sector_count as u64 * 512;
    if keep_len >= cap { return Ok(()); }
    let mut sector = keep_len / 512;
    let boundary_off = (keep_len % 512) as usize;
    if boundary_off != 0 {
        let lba = e.start_lba as u64 + sector;
        let rstatus = DiskFs::diskfs_block_read(lba * 512, 512, SLOT_BUF_LEND);
        if rstatus != 0 { return Err(rstatus); }
        unsafe {
            let p = buf_va as *mut u8;
            for k in boundary_off..512 { core::ptr::write_volatile(p.add(k), 0u8); }
        }
        let status = DiskFs::diskfs_block_write(lba * 512, 512, SLOT_BUF_LEND);
        if status != 0 { return Err(status); }
        sector += 1;
    }
    if sector * 512 < cap {
        unsafe {
            let p = buf_va as *mut u8;
            for k in 0..512 { core::ptr::write_volatile(p.add(k), 0u8); }
        }
        while sector * 512 < cap {
            let lba = e.start_lba as u64 + sector;
            let status = DiskFs::diskfs_block_write(lba * 512, 512, SLOT_BUF_LEND);
            if status != 0 { return Err(status); }
            sector += 1;
        }
    }
    Ok(())
}

/// Allocate `need_blocks` additional pool blocks, appending new extents to
/// `out` starting at index `n` (merging is not attempted — MAX_EXTENTS
/// gives ample headroom for the object-size cap in play). Returns the new
/// total extent count, or ERR_FULL if fragmentation or pool exhaustion
/// prevents satisfying the request.
fn v4_allocate(
    bm: &mut [u64; 3],
    need_blocks: u64,
    out: &mut [V4Extent; DISKFS_V4_MAX_EXTENTS],
    n: usize,
) -> Result<usize, i64> {
    // A multi-run allocation (pool fragmented enough that satisfying
    // need_blocks takes more than one contiguous stretch) can find and
    // commit several runs before a LATER run fails - either the pool runs
    // out of free blocks or `out` runs out of extent slots. Without the
    // rollback below, the earlier runs' bitmap bits would stay set despite
    // the overall call returning Err, leaking them until the next reboot's
    // bitmap rebuild (which recomputes from committed indirect descriptors
    // only, silently healing it - but same-session, real requests would
    // spuriously see less free space than actually exists). start_n marks
    // where this call's own runs begin in `out`, so on failure every run
    // this call itself set can be freed before returning.
    let start_n = n;
    let mut n = n;
    let mut need = need_blocks;
    let mut block = 0u64;
    while need > 0 {
        if block >= DISKFS_V4_POOL_BLOCKS {
            v4_free_pool_only(bm, &out[start_n..n]);
            return Err(messages::ERR_FULL);
        }
        if v4_bitmap_test(bm, block) { block += 1; continue; }
        let run_start = block;
        let mut run_len = 0u64;
        while block < DISKFS_V4_POOL_BLOCKS && !v4_bitmap_test(bm, block) && run_len < need {
            run_len += 1;
            block += 1;
        }
        if n >= DISKFS_V4_MAX_EXTENTS {
            v4_free_pool_only(bm, &out[start_n..n]);
            return Err(messages::ERR_FULL);
        }
        for b in 0..run_len { v4_bitmap_set(bm, run_start + b); }
        out[n] = V4Extent {
            start_lba: (DISKFS_V4_POOL_BASE_LBA + run_start * DISKFS_V4_BLOCK_SECTORS) as u16,
            sector_count: (run_len * DISKFS_V4_BLOCK_SECTORS) as u16,
        };
        n += 1;
        need -= run_len;
    }
    Ok(n)
}

/// Free extents back into the pool bitmap. Extents outside the pool
/// region (legacy migrated objects) are silently skipped — they were
/// never bitmap-tracked in the first place (see module doc).
fn v4_free_pool_only(bm: &mut [u64; 3], freed: &[V4Extent]) {
    let pool_end_lba = DISKFS_V4_POOL_BASE_LBA + DISKFS_V4_POOL_BLOCKS * DISKFS_V4_BLOCK_SECTORS;
    for e in freed {
        let start = e.start_lba as u64;
        let end = start + e.sector_count as u64;
        if start >= DISKFS_V4_POOL_BASE_LBA
            && end <= pool_end_lba
            && (start - DISKFS_V4_POOL_BASE_LBA) % DISKFS_V4_BLOCK_SECTORS == 0
            && e.sector_count as u64 % DISKFS_V4_BLOCK_SECTORS == 0
        {
            let b0 = (start - DISKFS_V4_POOL_BASE_LBA) / DISKFS_V4_BLOCK_SECTORS;
            let bn = e.sector_count as u64 / DISKFS_V4_BLOCK_SECTORS;
            for b in 0..bn { v4_bitmap_clear(bm, b0 + b); }
        }
    }
}

/// Rebuild the derived free/used bitmap from every in-use slot's indirect
/// descriptor. A slot whose descriptor is unreadable, overlaps another
/// slot's blocks, or claims less capacity than its own size_bytes needs is
/// dropped (not trusted) rather than mounted; the manifest is
/// re-persisted once if any drops occurred.
fn v4_bitmap_rebuild(buf_va: u64) {
    let mut bm = [0u64; 3];
    let mut dropped = false;
    let pool_end_lba = DISKFS_V4_POOL_BASE_LBA + DISKFS_V4_POOL_BLOCKS * DISKFS_V4_BLOCK_SECTORS;
    for i in 0..DISKFS_SLOTS {
        if !v4_in_use(i) { continue; }
        let size_bytes = unsafe { V4_TABLE[i].size_bytes } as u64;
        let (extents, count) = match v4_indirect_read(i, buf_va) {
            Ok(v) => v,
            // A fresh empty object need not have an indirect descriptor.
            // A valid descriptor MUST still reserve its blocks when size is
            // zero: a crash can land after growth publishes the descriptor
            // but before the manifest publishes size (or during truncate).
            // Later writes reuse that descriptor, so freeing those blocks
            // here would let another object allocate and overwrite them.
            Err(_) if size_bytes == 0 => continue,
            Err(_) => {
                crate::pdx::serial_println!(
                    "[sexfiles.diskfs.v4.rebuild.drop] slot={} reason=indirect_unreadable", i);
                unsafe { V4_TABLE[i] = V4_EMPTY; }
                dropped = true;
                continue;
            }
        };
        let mut ok = true;
        let mut total_bytes = 0u64;
        for k in 0..count {
            let e = extents[k];
            if e.sector_count == 0 { ok = false; break; }
            let start = e.start_lba as u64;
            let end = start + e.sector_count as u64;
            if sex_pdx::ranges_overlap(start,e.sector_count as u64,sex_pdx::SPINDLE_HISTORY_BASE_LBA,sex_pdx::SPINDLE_HISTORY_SECTORS){ok=false;break;}
            if start >= DISKFS_V4_POOL_BASE_LBA && end <= pool_end_lba {
                if (start - DISKFS_V4_POOL_BASE_LBA) % DISKFS_V4_BLOCK_SECTORS != 0
                    || e.sector_count as u64 % DISKFS_V4_BLOCK_SECTORS != 0
                {
                    ok = false; break;
                }
                let b0 = (start - DISKFS_V4_POOL_BASE_LBA) / DISKFS_V4_BLOCK_SECTORS;
                let bn = e.sector_count as u64 / DISKFS_V4_BLOCK_SECTORS;
                let mut bad = false;
                for b in 0..bn { if v4_bitmap_test(&bm, b0 + b) { bad = true; break; } }
                if bad { ok = false; break; }
                for b in 0..bn { v4_bitmap_set(&mut bm, b0 + b); }
            } else if end > DISKFS_V4_POOL_BASE_LBA && start < pool_end_lba {
                // Straddles a pool boundary — this allocator never
                // produces that; treat as corruption.
                ok = false; break;
            }
            // else: fully outside the pool (legacy/reserved) — not tracked.
            total_bytes += e.sector_count as u64 * 512;
        }
        if !ok || total_bytes < size_bytes {
            crate::pdx::serial_println!(
                "[sexfiles.diskfs.v4.rebuild.drop] slot={} reason=extent_overlap_or_short", i);
            unsafe { V4_TABLE[i] = V4_EMPTY; }
            dropped = true;
        }
    }
    *V4_BITMAP.write() = bm;
    if dropped {
        let _ = v4_persist(buf_va);
    }
}

/// Locate the extent (and offset within it) covering a logical byte
/// offset, from an in-hand extent array (no disk I/O).
fn v4_locate(extents: &[V4Extent; DISKFS_V4_MAX_EXTENTS], count: usize, logical_offset: u64) -> Result<(DiskManifestEntryV1, u64), u64> {
    let mut cum = 0u64;
    for k in 0..count {
        let e = extents[k];
        let cap = e.sector_count as u64 * 512;
        if logical_offset < cum + cap {
            return Ok((
                DiskManifestEntryV1 { name_hash: 0, start_lba: e.start_lba as u64, len_bytes: cap as u32, flags: 0 },
                logical_offset - cum,
            ));
        }
        cum += cap;
    }
    Err(messages::ERR_OVERFLOW as u64)
}

/// Read a slot's indirect descriptor from disk, then locate the extent
/// covering `logical_offset`. Used by the read path (no mutation in
/// flight, so re-reading is simplest).
fn v4_resolve_offset(i: usize, logical_offset: u64, buf_va: u64) -> Result<(DiskManifestEntryV1, u64), u64> {
    let (extents, count) = v4_cache_get(i, buf_va);
    v4_locate(&extents, count, logical_offset)
}

fn v4_get(path_id: u64) -> Result<V4Entry, u64> {
    if path_id >= DISKFS_SLOTS as u64 { return Err(messages::ERR_BAD_CMD as u64); }
    let i = path_id as usize;
    if !v4_in_use(i) { return Err(messages::ERR_NOT_FOUND as u64); }
    Ok(unsafe { V4_TABLE[i] })
}

fn v4_persist(buf_va: u64) -> Result<(), u64> {
    let mut sector = [0u8; 512];
    sector[0..8].copy_from_slice(&DISKFS_MANIFEST_MAGIC.to_le_bytes());
    sector[8..10].copy_from_slice(&DISKFS_V4_VERSION.to_le_bytes());
    sector[10..12].copy_from_slice(&(DISKFS_SLOTS as u16).to_le_bytes());
    let g = V4_GENERATION.fetch_add(1, Ordering::Relaxed) as u32 + 1;
    sector[12..16].copy_from_slice(&g.to_le_bytes());
    for i in 0..DISKFS_SLOTS {
        let e = unsafe { V4_TABLE[i] };
        let off = 16 + i * 32;
        sector[off..off + DISKFS_V3_NAME_MAX].copy_from_slice(&e.name);
        sector[off + 24..off + 28].copy_from_slice(&e.size_bytes.to_le_bytes());
        sector[off + 28..off + 30].copy_from_slice(&v4_checksum_entry(&e).to_le_bytes());
        sector[off + 30..off + 32].copy_from_slice(&e.gen.to_le_bytes());
    }
    unsafe {
        let p = buf_va as *mut u8;
        for k in 0..512 { core::ptr::write_volatile(p.add(k), sector[k]); }
    }
    let status = DiskFs::diskfs_block_write(DISKFS_MANIFEST_LBA * 512, 512, SLOT_BUF_LEND);
    if status != 0 { return Err(status); }
    // Read-back verify.
    let mut check = [0u8; 512];
    if DiskFs::diskfs_block_read(DISKFS_MANIFEST_LBA * 512, 512, SLOT_BUF_LEND) != 0 {
        return Err(messages::ERR_BAD_CMD as u64);
    }
    unsafe {
        let p = buf_va as *const u8;
        for k in 0..512 { check[k] = core::ptr::read_volatile(p.add(k)); }
    }
    if check != sector {
        crate::pdx::serial_println!("[sexfiles.diskfs.v4.persist.err] reason=verify_mismatch");
        return Err(messages::ERR_BAD_CMD as u64);
    }
    crate::pdx::serial_println!("[sexfiles.diskfs.v4.persist.ok] generation={}", g);
    Ok(())
}

/// Load (or migrate/bootstrap) the V4 manifest. Idempotent; cached.
/// Boot-time confirmation that the content pool's resolved LBA range
/// doesn't intersect any reserved region — sex-pdx's `const _: () =
/// assert!(...)` block already makes this impossible to build if false,
/// so this can never actually fail; it exists as a live, gate-greppable
/// marker (`[sexfiles.diskfs.v4.layout.ok]` / a loud `.layout.fail`) that
/// the safe layout is really what's running, not just what compiled — see
/// docs/handoff/SEXDRIVE_NVME_QUEUE_WRAP_V1.md for why "it compiled" and
/// "it's actually safe" turned out to be two different claims once before.
fn v4_layout_selfcheck() {
    let pool_end = DISKFS_V4_POOL_BASE_LBA + DISKFS_V4_POOL_BLOCKS * DISKFS_V4_BLOCK_SECTORS;
    let reserved: [(&str, u64, u64); 7] = [
        ("spindle_history",sex_pdx::SPINDLE_HISTORY_BASE_LBA,sex_pdx::SPINDLE_HISTORY_SECTORS),
        ("sexdrive_ap3", sex_pdx::SEXDRIVE_AP3_WRITE_PROOF_LBA, sex_pdx::SEXDRIVE_AP3_WRITE_PROOF_SECTORS),
        ("sexdrive_ap4", sex_pdx::SEXDRIVE_AP4_MULTI_BASE_LBA, sex_pdx::SEXDRIVE_AP4_MULTI_SECTORS),
        ("sexdrive_ap5a", sex_pdx::SEXDRIVE_AP5A_PERSIST_BASE_LBA, sex_pdx::SEXDRIVE_AP5A_PERSIST_SECTORS),
        ("sexdrive_ap6", sex_pdx::SEXDRIVE_AP6_NEG_MISMATCH_LBA, sex_pdx::SEXDRIVE_AP6_NEG_MISMATCH_SECTORS),
        ("diskfs_manifest", sex_pdx::DISKFS_MANIFEST_LBA, sex_pdx::DISKFS_MANIFEST_SECTORS),
        ("diskfs_legacy_slots", sex_pdx::DISKFS_LEGACY_SLOTS_START_LBA, sex_pdx::DISKFS_LEGACY_SLOTS_SECTORS),
    ];
    let mut ok = true;
    for (name, start, len) in reserved {
        if sex_pdx::ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_BLOCKS * DISKFS_V4_BLOCK_SECTORS, start, len) {
            crate::pdx::serial_println!(
                "[sexfiles.diskfs.v4.layout.fail] pool={}..{} collides_with={} region={}..{}",
                DISKFS_V4_POOL_BASE_LBA, pool_end, name, start, start + len
            );
            ok = false;
        }
    }
    if ok {
        crate::pdx::serial_println!(
            "[sexfiles.diskfs.v4.layout.ok] pool={}..{} indirect_top={} manifest={}",
            DISKFS_V4_POOL_BASE_LBA, pool_end, DISKFS_V4_INDIRECT_BASE_LBA, DISKFS_MANIFEST_LBA
        );
    }
}

fn v4_ensure(buf_va: u64) -> Result<(), u64> {
    if V4_LOADED.load(Ordering::Relaxed) != 0 { return Ok(()); }
    v4_layout_selfcheck();
    let mut sector = [0u8; 512];
    if DiskFs::diskfs_block_read(DISKFS_MANIFEST_LBA * 512, 512, SLOT_BUF_LEND) != 0 {
        return Err(messages::ERR_BAD_CMD as u64);
    }
    unsafe {
        let p = buf_va as *const u8;
        for k in 0..512 { sector[k] = core::ptr::read_volatile(p.add(k)); }
    }
    let magic = u64::from_le_bytes(sector[0..8].try_into().unwrap());
    let version = u16::from_le_bytes(sector[8..10].try_into().unwrap());
    if magic == DISKFS_MANIFEST_MAGIC && version == DISKFS_V4_VERSION {
        let g = u32::from_le_bytes(sector[12..16].try_into().unwrap());
        V4_GENERATION.store(g as u64, Ordering::Relaxed);
        let mut live = 0;
        for i in 0..DISKFS_SLOTS {
            let off = 16 + i * 32;
            let mut e = V4_EMPTY;
            e.name.copy_from_slice(&sector[off..off + DISKFS_V3_NAME_MAX]);
            e.size_bytes = u32::from_le_bytes(sector[off + 24..off + 28].try_into().unwrap());
            let stored_cs = u16::from_le_bytes(sector[off + 28..off + 30].try_into().unwrap());
            e.gen = u16::from_le_bytes(sector[off + 30..off + 32].try_into().unwrap());
            if e.name[0] != 0 && v4_checksum_entry(&e) != stored_cs {
                crate::pdx::serial_println!(
                    "[sexfiles.diskfs.v4.load.drop] slot={} reason=checksum", i);
                e = V4_EMPTY;
            }
            if e.name[0] != 0 { live += 1; }
            unsafe { V4_TABLE[i] = e; }
        }
        V4_LOADED.store(1, Ordering::Relaxed);
        crate::pdx::serial_println!("[sexfiles.diskfs.v4.load.ok] live={} generation={}", live, g);
        v4_bitmap_rebuild(buf_va);
        return Ok(());
    }
    // Migration / bootstrap only fires for the two SAFE cases:
    //  - magic matches (this is genuinely our manifest format) but version
    //    is an older, recognized predecessor -> real migration.
    //  - the sector is all-zero -> genuinely unformatted disk.
    // Anything else (non-matching magic on a non-blank sector: torn write,
    // bad sector, foreign data) is corruption, not "unknown, so start
    // fresh" - silently overwriting it would destroy whatever was really
    // there. Refuse to mount instead; every v4_ensure caller already
    // propagates Err(e) as a hard error, so this fails visibly rather than
    // panicking or exposing a partially-valid state.
    let recognized_legacy = magic == DISKFS_MANIFEST_MAGIC && version < DISKFS_V4_VERSION;
    let genuinely_blank = sector.iter().all(|&b| b == 0);
    if !recognized_legacy && !genuinely_blank {
        crate::pdx::serial_println!(
            "[sexfiles.diskfs.v4.mount.err] reason=corrupt_manifest magic={:#x} version={}",
            magic, version
        );
        return Err(messages::ERR_CORRUPT as u64);
    }
    // Migration / bootstrap: recognized V3 manifest -> wrap the 3 legacy
    // system objects as single-extent V4 entries at their EXISTING
    // physical LBAs (no data movement). Genuinely blank disk bootstraps
    // fresh at the same legacy layout V3 originally used.
    let legacy: [(&[u8], u16); 3] = [
        (b"sexfiles-proof-v1", 2038u16),
        (b"linen-object-v1", 2030u16),
        (b"quil-object-v1", 2022u16),
    ];
    for i in 0..DISKFS_SLOTS {
        if i < legacy.len() {
            let (nm, lba) = legacy[i];
            let mut e = V4_EMPTY;
            e.name[..nm.len()].copy_from_slice(nm);
            e.size_bytes = 4096; // V3 objects were always fully-sized 4096B
            e.gen = 1;
            unsafe { V4_TABLE[i] = e; }
            let extents = [V4Extent { start_lba: lba, sector_count: 8 }];
            if let Err(er) = v4_indirect_write(i, buf_va, &extents, 1) {
                crate::pdx::serial_println!("[sexfiles.diskfs.v4.migrate.err] slot={} err={}", i, er);
                return Err(er);
            }
        } else {
            unsafe { V4_TABLE[i] = V4_EMPTY; }
        }
    }
    v4_persist(buf_va)?;
    V4_LOADED.store(1, Ordering::Relaxed);
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.migrate.ok] from_version={} seeded=3", version);
    v4_bitmap_rebuild(buf_va);
    Ok(())
}

fn v4_unpack_name(lo: u64, hi: u64) -> ([u8; DISKFS_V3_NAME_MAX], usize) {
    let mut name = [0u8; DISKFS_V3_NAME_MAX];
    name[..8].copy_from_slice(&lo.to_le_bytes());
    name[8..16].copy_from_slice(&hi.to_le_bytes());
    // Sanitize: printable ASCII only, stop at first NUL.
    let mut n = 0;
    while n < 16 && name[n] != 0 {
        if name[n] < 0x20 || name[n] > 0x7E { name[n] = b'_'; }
        n += 1;
    }
    for b in name[n..].iter_mut() { *b = 0; }
    (name, n)
}

/// OP_DISKFS_CREATE (0x42): arg1|arg2 = up to 16 name bytes packed LE.
/// Returns new path_id. Duplicate name → ERR_EXISTS; no name → ERR_BAD_CMD;
/// table full → ERR_FULL. The new object starts empty (size_bytes=0, no
/// extents) — there is nothing to zero because there is no backing storage
/// until the first WRITE, which always zero-fills newly allocated blocks.
fn handle_diskfs_create(name_lo: u64, name_hi: u64, buf_va: u64) -> u64 {
    let (name, n) = v4_unpack_name(name_lo, name_hi);
    if n == 0 { return messages::ERR_BAD_CMD as u64; }
    for i in 0..DISKFS_SLOTS {
        if v4_in_use(i) && unsafe { V4_TABLE[i].name } == name {
            crate::pdx::serial_println!(
                "[sexfiles.diskfs.v4.create.err] reason=exists slot={}", i);
            return messages::ERR_EXISTS as u64;
        }
    }
    let mut slot = None;
    for i in (DISKFS_V3_SYSTEM_SLOTS as usize)..DISKFS_SLOTS {
        if !v4_in_use(i) && unsafe{V4_TABLE[i].gen}<u16::MAX { slot = Some(i); break; }
    }
    let Some(i) = slot else {
        crate::pdx::serial_println!("[sexfiles.diskfs.v4.create.err] reason=full");
        return messages::ERR_FULL as u64;
    };
    // A deleted slot can retain a valid descriptor if deletion crashed after
    // its manifest commit. Clear that old identity's extents before publishing
    // the new live slot, or recovery could reserve blocks now owned elsewhere.
    if let Err(er) = v4_indirect_write(i, buf_va, &[], 0) { return er; }
    v4_cache_invalidate(i);
    let Some(gen) = unsafe { V4_TABLE[i].gen }.checked_add(1)else{return messages::ERR_OVERFLOW as u64;};
    let e = V4Entry { name, size_bytes: 0, gen };
    unsafe { V4_TABLE[i] = e; }
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.crash_point.create_pending] slot={} gen={}", i, gen);
    if let Err(er) = v4_persist(buf_va) {
        unsafe { V4_TABLE[i] = V4_EMPTY; }
        return er;
    }
    crate::pdx::serial_println!("[sexfiles.diskfs.v4.create.ok] slot={} gen={}", i, gen);
    i as u64
}

/// OP_DISKFS_LIST (0x43): arg0 = path_id, arg1 = query.
///   query 0/1/2 → 8 name bytes (chunk) packed LE (0 = free slot / past end)
///   query 0xFF  → (in_use<<62) | (gen<<32) | slot_count (bit63 kept clear:
///                 sync clients sign-check replies)
///   query 0xFE  → global manifest generation (change detection for Linen)
fn handle_diskfs_list(path_id: u64, query: u64) -> u64 {
    if query == 0xFE {
        return V4_GENERATION.load(Ordering::Relaxed);
    }
    if query == 0xFF {
        if path_id >= DISKFS_SLOTS as u64 { return 0; }
        let i = path_id as usize;
        let used = v4_in_use(i) as u64;
        let gen = unsafe { V4_TABLE[i].gen } as u64;
        // bit62 (NOT 63): reply values are sign-checked by sync clients.
        return (used << 62) | (gen << 32) | (DISKFS_SLOTS as u64);
    }
    if path_id >= DISKFS_SLOTS as u64 || !v4_in_use(path_id as usize) { return 0; }
    let name = unsafe { V4_TABLE[path_id as usize].name };
    let chunk = (query as usize).min(2);
    let mut v = 0u64;
    for k in 0..8 {
        let idx = chunk * 8 + k;
        if idx < DISKFS_V3_NAME_MAX {
            v |= (name[idx] as u64) << (k * 8);
        }
    }
    v
}

/// OP_DISKFS_DELETE (0x47): arg0 = path_id. System slots protected. Frees
/// the object's extents from the live bitmap immediately (same-boot reuse,
/// not just after reboot), then invalidates the indirect descriptor.
fn handle_diskfs_delete(path_id: u64, buf_va: u64) -> u64 {
    if document_locked(path_id) { return messages::ERR_FULL as u64; }
    if path_id < DISKFS_V3_SYSTEM_SLOTS { return messages::ERR_PERM_DENIED as u64; }
    if path_id >= DISKFS_SLOTS as u64 { return messages::ERR_BAD_CMD as u64; }
    let i = path_id as usize;
    if !v4_in_use(i) { return messages::ERR_NOT_FOUND as u64; }
    let (extents, count) = v4_indirect_read(i, buf_va).unwrap_or(([V4Extent::default(); DISKFS_V4_MAX_EXTENTS], 0));
    let old_entry = unsafe { V4_TABLE[i] };
    let Some(next_gen)=old_entry.gen.checked_add(1)else{return messages::ERR_OVERFLOW as u64;};
    unsafe {
        V4_TABLE[i].name = [0u8; DISKFS_V3_NAME_MAX];
        V4_TABLE[i].size_bytes = 0;
        V4_TABLE[i].gen = next_gen;
    }
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.crash_point.delete_pending] slot={}", i);
    if let Err(e) = v4_persist(buf_va) {
        // A failed delete must not expose this still-owned slot to CREATE,
        // which resets descriptors before publishing a new object identity.
        unsafe { V4_TABLE[i] = old_entry; }
        return e;
    }
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.crash_point.delete_committed] slot={}", i);
    {
        let mut bm = V4_BITMAP.write();
        v4_free_pool_only(&mut bm, &extents[..count]);
    }
    let _ = v4_indirect_write(i, buf_va, &[], 0);
    v4_cache_invalidate(i);
    crate::pdx::serial_println!("[sexfiles.diskfs.v4.delete.ok] slot={}", i);
    0
}

/// OP_DISKFS_RENAME (0x48): arg0 = path_id, arg1|arg2 = new name.
fn handle_diskfs_rename(path_id: u64, name_lo: u64, name_hi: u64, buf_va: u64) -> u64 {
    if document_locked(path_id) { return messages::ERR_FULL as u64; }
    if path_id < DISKFS_V3_SYSTEM_SLOTS { return messages::ERR_PERM_DENIED as u64; }
    if path_id >= DISKFS_SLOTS as u64 { return messages::ERR_BAD_CMD as u64; }
    let i = path_id as usize;
    if !v4_in_use(i) { return messages::ERR_NOT_FOUND as u64; }
    let (name, n) = v4_unpack_name(name_lo, name_hi);
    if n == 0 { return messages::ERR_BAD_CMD as u64; }
    for j in 0..DISKFS_SLOTS {
        if j != i && v4_in_use(j) && unsafe { V4_TABLE[j].name } == name {
            return messages::ERR_EXISTS as u64;
        }
    }
    unsafe { V4_TABLE[i].name = name; }
    crate::pdx::serial_println!(
        "[sexfiles.diskfs.v4.crash_point.rename_pending] slot={}", i);
    if let Err(e) = v4_persist(buf_va) { return e; }
    crate::pdx::serial_println!("[sexfiles.diskfs.v4.rename.ok] slot={}", i);
    0
}

pub fn handle_vfs_message(type_id: u64, arg0: u64, arg1: u64, arg2: u64, caller_pd: u32) -> u64 {
    crate::pdx::serial_println!(
        "[sexfiles.vfs.enter] type={:#x} caller={} a0={:#x} a1={:#x}",
        type_id, caller_pd, arg0, arg1
    );
    IPC_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);

    // All operations currently route to RamFS.
    let backend: &dyn FsBackend = &RAMFS;

    match type_id {
        // ── OP_RAMFS_OPEN ──
        // arg0 = name[0..7], arg1 = name[8..15], arg2 = name[16..23] | (flags << 24)
        messages::OP_RAMFS_OPEN => {
            // Mask flag byte (bits 24-31) from name portion per protocol spec:
            //   arg2 = name[16..23] | (flags << 24)
            let name_bytes = unpack_name(arg0, arg1, arg2 & !(0xFFu64 << 24));
            let flags = (arg2 >> 24) as u32;
            match backend.open(&name_bytes, flags, 0, caller_pd) {
                Ok(handle) => handle,
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_READ ──
        // arg0 = handle, arg1 = offset, arg2 = max_len
        messages::OP_RAMFS_READ => {
            let handle = arg0;
            let offset = arg1;
            let max_len = (arg2 as usize).min(messages::RAMFS_MAX_FILE_SIZE);
            let mut buf = [0u8; 8]; // Return up to 8 bytes in the reply
            let to_read = max_len.min(buf.len());
            match backend.read(handle, offset, &mut buf[..to_read], caller_pd) {
                Ok(n) => {
                    // Pack read data into reply u64
                    let mut reply = 0u64;
                    for i in 0..n.min(8) as usize {
                        reply |= (buf[i] as u64) << (i * 8);
                    }
                    reply
                }
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_WRITE ──
        // arg0 = handle, arg1 = offset, arg2 = packed data (8 bytes)
        messages::OP_RAMFS_WRITE => {
            let handle = arg0;
            let offset = arg1;
            let data = arg2.to_le_bytes(); // 8 bytes of data
            match backend.write(handle, offset, &data, caller_pd) {
                Ok(n) => n,
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_CLOSE ──
        // arg0 = handle
        messages::OP_RAMFS_CLOSE => {
            let handle = arg0;
            match backend.close(handle, caller_pd) {
                Ok(_) => 0,
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_LIST ──
        // arg0 = index
        // Returns: packed { handle: u64, name_len: u32 } in upper/lower bits,
        // or 0 if no more entries. Only returns entries owned by caller_pd.
        messages::OP_RAMFS_LIST => {
            let index = arg0 as usize;
            match backend.list_at(index, caller_pd) {
                Some((handle, name_len)) => {
                    (handle << 32) | (name_len as u64)
                }
                None => 0,
            }
        }

        // ── OP_RAMFS_CREATE_OWNER ──
        // arg0 = name bytes 0..7
        // arg1 = name bytes 8..15
        // arg2 = name bytes 16..23 (lower 24 bits) | (owner_pd << 32)
        messages::OP_RAMFS_CREATE_OWNER => {
            let name_bytes =
                unpack_name(arg0, arg1, arg2 & !(0xFFFF_FFFFu64 << 32));
            let owner_pd = (arg2 >> 32) as u32;
            match backend.create_with_owner(&name_bytes, owner_pd, caller_pd) {
                Ok(handle) => handle,
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_OBJECT_ID ──
        // arg0 = handle
        // Returns: RamFS-assigned object_id (≥1) for the open handle.
        // Caller must own the file (or caller_pd=0 for server-internal).
        // Use to resolve OQ5: obtain SexFiles-assigned global ID from a handle.
        messages::OP_RAMFS_OBJECT_ID => {
            let handle = arg0;
            match RAMFS.object_id_for_handle(handle, caller_pd) {
                Ok(id) => id,
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_STAT ──
        // arg0 = handle
        messages::OP_RAMFS_STAT => {
            let handle = arg0;
            match backend.stat(handle, caller_pd) {
                Ok((size, name_len)) => {
                    (size << 32) | name_len as u64
                }
                Err(e) => e as u64,
            }
        }

        // ── OP_RAMFS_READNAME ──
        // arg0 = handle, arg1 = byte_offset, arg2 = max_len (clamped to 8)
        // Returns: up to 8 filename bytes LE. 0 = EOF. negative = error.
        messages::OP_RAMFS_READNAME => {
            let handle = arg0;
            let byte_offset = arg1;
            let max_len = (arg2 as usize).min(8);
            let mut buf = [0u8; 8];
            match RAMFS.read(handle, byte_offset, &mut buf[..max_len], caller_pd) {
                Ok(n) => {
                    let mut packed = 0u64;
                    for i in 0..(n as usize).min(8) {
                        packed |= (buf[i] as u64) << (i * 8);
                    }
                    crate::pdx::serial_println!(
                        "[sexfiles.ramfs.readname.ok] handle={} off={} len={}",
                        handle, byte_offset, max_len
                    );
                    packed
                }
                Err(e) => {
                    crate::pdx::serial_println!(
                        "[sexfiles.ramfs.readname.deny] handle={} err={}",
                        handle, e
                    );
                    e as u64
                }
            }
        }

        // ── DiskFS bridge opcodes (0x38-0x3C) ──
        // Route: Linen → SLOT_STORAGE → SexFiles → DiskFS → SLOT_BLOCK → SexDrive → NVMe
        // Fixed-object bridge: /disk/sexfiles-proof-v1
        messages::OP_DISKFS_WRITE => {
            // arg0 = byte_offset, arg1 = data_lo, arg2 = data_hi
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x38 name=write caller={}", caller_pd); }
            let reply = handle_diskfs_write(arg0, arg1, arg2, caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x38 caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_READ => {
            // arg0 = byte_offset, arg1 = max_len, arg2 = 0 (reserved)
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x39 name=read caller={}", caller_pd); }
            let reply = handle_diskfs_read(arg0, arg1, caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x39 caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_FLUSH => {
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x3A name=flush caller={}", caller_pd); }
            let reply = handle_diskfs_flush();
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x3A caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_STAT => {
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x3B name=stat caller={}", caller_pd); }
            let reply = handle_diskfs_stat(caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x3B caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_MANIFEST_HASH => {
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x3C name=hash caller={}", caller_pd); }
            let reply = handle_diskfs_manifest_hash(caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x3C caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_CREATE => {
            crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x42 name=create caller={}", caller_pd);
            let buf_va = diskfs_bridge_get_buf_va();
            if buf_va == 0 || buf_va == u64::MAX { return messages::ERR_NOT_FOUND as u64; }
            if let Err(e) = v4_ensure(buf_va) { return e; }
            let reply = handle_diskfs_create(arg1, arg2, buf_va);
            crate::pdx::serial_println!("[sexfiles.route.reply] op=0x42 caller={} value={:#x}", caller_pd, reply);
            reply
        }
        messages::OP_DISKFS_LIST => {
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x43 name=list caller={}", caller_pd); }
            let buf_va = diskfs_bridge_get_buf_va();
            if buf_va == 0 || buf_va == u64::MAX { return messages::ERR_NOT_FOUND as u64; }
            if let Err(e) = v4_ensure(buf_va) { return e; }
            handle_diskfs_list(arg0, arg1)
        }
        messages::OP_DISKFS_DELETE => {
            crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x47 name=delete caller={}", caller_pd);
            let buf_va = diskfs_bridge_get_buf_va();
            if buf_va == 0 || buf_va == u64::MAX { return messages::ERR_NOT_FOUND as u64; }
            if let Err(e) = v4_ensure(buf_va) { return e; }
            handle_diskfs_delete(arg0, buf_va)
        }
        messages::OP_DISKFS_RENAME => {
            crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x48 name=rename caller={}", caller_pd);
            let buf_va = diskfs_bridge_get_buf_va();
            if buf_va == 0 || buf_va == u64::MAX { return messages::ERR_NOT_FOUND as u64; }
            if let Err(e) = v4_ensure(buf_va) { return e; }
            handle_diskfs_rename(arg0, arg1, arg2, buf_va)
        }
        messages::OP_DISKFS_TRUNCATE => {
            // arg0 = new_length_bytes
            crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x49 name=truncate caller={}", caller_pd);
            let reply = handle_diskfs_truncate(arg0, caller_pd);
            crate::pdx::serial_println!("[sexfiles.route.reply] op=0x49 caller={} value={:#x}", caller_pd, reply);
            reply
        }
        messages::OP_DISKFS_READ_V2 => {
            // arg0 = byte_offset, arg1 = want_len, arg2 = 0 (reserved)
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x4A name=read_v2 caller={}", caller_pd); }
            let reply = handle_diskfs_read_v2(arg0, arg1, caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x4A caller={} value={:#x}", caller_pd, reply); }
            reply
        }
        messages::OP_DISKFS_SELECT => {
            // arg0 = path_id, arg1/arg2 = 0 (reserved)
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.dispatch] op=0x3E name=select caller={}", caller_pd); }
            let reply = handle_diskfs_select(arg0, caller_pd);
            if hot_log() { crate::pdx::serial_println!("[sexfiles.route.reply] op=0x3E caller={} value={:#x}", caller_pd, reply); }
            reply
        }

        messages::OP_SEXOBJECT_NATIVE_PERSIST_PROOF => {
            if hot_log() { crate::pdx::serial_println!(
                "[sexfiles.route.dispatch] op=0x40 ok=1 caller={}",
                caller_pd,
            ); }
            match crate::backends::diskfs::sexobject_native_persist_linen_proof() {
                Ok(object_id) => {
                    if hot_log() { crate::pdx::serial_println!(
                        "[sexfiles.route.reply] op=0x40 status=S ok=1 caller={} object_id={}",
                        caller_pd, object_id,
                    ); }
                    object_id
                }
                Err(e) => {
                    if hot_log() { crate::pdx::serial_println!(
                        "[sexfiles.route.reply] op=0x40 status=E ok=0 caller={} err={}",
                        caller_pd, e,
                    ); }
                    e as u64
                }
            }
        }

        messages::OP_SEXOBJECT_READ_BACK => {
            // arg0 = object_id, arg1/arg2 = 0 (reserved)
            if hot_log() { crate::pdx::serial_println!(
                "[sexfiles.route.dispatch] op=0x41 name=sexobject_read_back caller={}",
                caller_pd
            ); }
            match crate::backends::diskfs::sexobject_read_back_for_quil(arg0) {
                Ok(len) => {
                    if hot_log() { crate::pdx::serial_println!(
                        "[sexfiles.route.reply] op=0x41 caller={} len={} ok=1",
                        caller_pd, len
                    ); }
                    // Return len as positive value for the caller to verify
                    len as u64
                }
                Err(e) => {
                    if hot_log() { crate::pdx::serial_println!(
                        "[sexfiles.route.reply] op=0x41 caller={} err={}",
                        caller_pd, e
                    ); }
                    e as u64
                }
            }
        }

        messages::OP_RAMFS_STATUS => {
            // Phase B1: object status query by object_id
            // arg0 = object_id, arg1/arg2 = 0
            crate::pdx::serial_println!("[sexfiles.status.query] object={} ok=1 reason=received", arg0);
            // Query backend for object existence
            // RamFS backend: check if any file has this object_id
            let exists = false; // stub — full lookup deferred
            let size: u64 = 0;
            let generation: u64 = 0;
            crate::pdx::serial_println!("[sexfiles.status.result] object={} exists={} size={} generation={} ok=1 reason=phaseb1_marker",
                arg0, exists as u8, size, generation);
            0 // fire-and-forget, no meaningful reply
        }

        _ => messages::ERR_NOT_FOUND as u64,
    }
}

/// Unpack a name from three u64 args.
/// Name is stored little-endian in arg0..arg2, up to 24 bytes,
/// zero-padded. Returns the actual name slice (strips trailing zeros).
fn unpack_name(arg0: u64, arg1: u64, arg2: u64) -> alloc::vec::Vec<u8> {
    let mut name = alloc::vec::Vec::with_capacity(messages::RAMFS_MAX_NAME);
    let bytes0 = arg0.to_le_bytes();
    let bytes1 = arg1.to_le_bytes();
    let bytes2 = arg2.to_le_bytes();
    name.extend_from_slice(&bytes0);
    name.extend_from_slice(&bytes1);
    // First 24 bytes of arg2 are name; rest may be flags
    name.extend_from_slice(&bytes2[..8]);
    // Strip trailing zeros (but preserve embedded zeros as valid name bytes)
    while name.last() == Some(&0) {
        name.pop();
    }
    name
}

/// Tagged Spindle diagnostics retain the existing bridge's BOOT route scope.
/// No caller is substituted and no object number mints authority.
pub struct SpindleProbeBackend;
impl crate::spindle_probe::Backend for SpindleProbeBackend{
 fn invoke(&mut self,caller:u32,r:sex_pdx::spindle_probe::Request)->Result<u32,u32>{
  use sex_pdx::spindle_probe as w;

  if r.object>=15||r.offset>w::MAX_BYTES{return Err(w::INVALID)}
  let number=|v:u64|if(v as i64)<0{Err(w::IO)}else{u32::try_from(v).map_err(|_|w::INVALID)};
  let select=|id:u8|if handle_diskfs_select(id as u64,caller)==0{Ok(())}else{Err(w::IO)};
  match r.kind{
   w::STAT|w::BEGIN=>{if r.offset!=0||r.a!=0||r.b!=0{return Err(w::INVALID)}select(r.object)?;let v=handle_diskfs_stat(caller);if(v as i64)<0{return Err(w::IO)}Ok(v as u32)},
   w::GENERATION|w::INFO|w::NAME=>{
    if r.a!=0||r.b!=0{return Err(w::INVALID)}
    let query=match r.kind{w::GENERATION if r.object==0&&r.offset==0=>0xFE,w::INFO if r.offset==0=>0xFF,w::NAME if r.offset<6=>(r.offset/2)as u64,_=>return Err(w::INVALID)};
    let v=handle_vfs_message(messages::OP_DISKFS_LIST,r.object as u64,query,0,caller);if(v as i64)<0{return Err(w::IO)}
    match r.kind{w::GENERATION if v<u32::MAX as u64=>Ok(v as u32),w::INFO=>Ok(((v>>62)&1)as u32),w::NAME=>Ok((v>>((r.offset%2)*32))as u32),_=>Err(w::STALE)}
   },
   w::READ=>{if !(1..=4).contains(&r.a)||r.b!=0||r.offset as u64+r.a>w::MAX_BYTES as u64{return Err(w::INVALID)}select(r.object)?;let v=handle_diskfs_read_v2(r.offset as u64,r.a,caller);if sex_pdx::diskfs_v2_status(v)!=sex_pdx::DISKFS_V2_STATUS_OK||sex_pdx::diskfs_v2_len_field(v)!=r.a{return Err(w::IO)}let bytes=sex_pdx::diskfs_v2_payload(v);Ok(u32::from_le_bytes(bytes[..4].try_into().unwrap()))},
   w::WRITE=>{if r.object<3||r.offset%16!=0||r.offset>w::MAX_BYTES-16{return Err(w::INVALID)}select(r.object)?;number(handle_diskfs_write(r.offset as u64,r.a,r.b,caller))},
   w::TRUNCATE=>{if r.object<3||r.offset!=0||r.a>w::MAX_BYTES as u64||r.b!=0{return Err(w::INVALID)}select(r.object)?;number(handle_diskfs_truncate(r.a,caller))},
   w::CREATE|w::RENAME=>{if r.offset!=0||r.kind==w::CREATE&&r.object!=0||r.kind==w::RENAME&&r.object<3{return Err(w::INVALID)}
    let(name,n)=v4_unpack_name(r.a,r.b);let mut raw=[0u8;16];raw[..8].copy_from_slice(&r.a.to_le_bytes());raw[8..].copy_from_slice(&r.b.to_le_bytes());if n==0||name[..16]!=raw{return Err(w::INVALID)}
    let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}v4_ensure(va).map_err(|_|w::IO)?;
    number(if r.kind==w::CREATE{handle_diskfs_create(r.a,r.b,va)}else{handle_diskfs_rename(r.object as u64,r.a,r.b,va)})
   },
   w::DELETE=>{if r.object<3||r.offset!=0||r.a!=0||r.b!=0{return Err(w::INVALID)}let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}v4_ensure(va).map_err(|_|w::IO)?;number(handle_diskfs_delete(r.object as u64,va))},
   w::SYNC=>{if r.offset!=0||r.a!=0||r.b!=0{return Err(w::INVALID)}number(crate::backends::diskfs::DiskFs::diskfs_block_sync())},
   _=>Err(w::INVALID)
  }
 }
}

/// Authenticated Quil BOOT subjects may select documents in the existing
/// shared workspace (legacy document 2 and user slots 3..14). No global
/// legacy selection is consulted by this adapter.
// Serialization is service-local transaction custody, not new authority.
// A live commit or recovery locks its existing V4 object against mutations.
static DOCUMENT_WRITE_LOCK: AtomicU64 = AtomicU64::new(0);
struct DocumentJob { caller:u32, object:u8, identity:u32, journal:crate::document_journal::Save, growth:Option<DocumentGrowth> }
static mut DOCUMENT_JOB:Option<DocumentJob> = None;
struct DocumentLoadJob { caller:u32, object:u8, identity:u32, journal:crate::document_journal::Load }
static mut DOCUMENT_LOAD_JOB:Option<DocumentLoadJob> = None;
fn document_locked(id:u64)->bool {
    let lock=DOCUMENT_WRITE_LOCK.load(Ordering::Relaxed);
    lock!=0 && id.checked_add(1)==Some(lock)
}
/// Reserved runs remain private until zeroing and FLUSH complete. Once a
/// descriptor write is attempted, retain its blocks on uncertainty: recovery
/// may already discover them even when the manifest still has its old size.
struct DocumentGrowth {
    id:usize, extents:[V4Extent;DISKFS_V4_MAX_EXTENTS], original:usize,
    count:usize, extent:usize, sector:u64, phase:u8, descriptor_attempted:bool,
}
impl DocumentGrowth {
    fn new(id:usize,va:u64)->Result<Self,u32>{
        use sex_pdx::document as w;
        let (mut extents,original)=v4_cache_get(id,va);
        let capacity:u64=extents[..original].iter().map(|e|e.sector_count as u64*512).sum();
        if capacity>crate::document_journal::CAPACITY{return Err(w::INVALID)}
        let blocks=(crate::document_journal::CAPACITY-capacity+4095)/4096;
        let count=v4_allocate(&mut V4_BITMAP.write(),blocks,&mut extents,original).map_err(|_|w::IO)?;
        Ok(Self{id,extents,original,count,extent:original,sector:0,phase:0,descriptor_attempted:false})
    }
    fn cancel(&self){
        if !self.descriptor_attempted {
            v4_free_pool_only(&mut V4_BITMAP.write(),&self.extents[self.original..self.count]);
        }
        v4_cache_invalidate(self.id);
    }
    fn step(&mut self,va:u64)->Result<bool,u32>{
        use sex_pdx::document as w;
        match self.phase {
            0=>{
                if self.extent<self.count {
                    let e=self.extents[self.extent];
                    v4_zero_block(e.start_lba as u64+self.sector,va).map_err(|_|w::IO)?;
                    self.sector+=DISKFS_V4_BLOCK_SECTORS;
                    if self.sector>=e.sector_count as u64{self.extent+=1;self.sector=0;}
                }else{self.phase=1;}
            }
            1=>{if DiskFs::diskfs_block_sync()!=0{return Err(w::IO)}self.phase=2;}
            2=>{
                self.descriptor_attempted=true;
                v4_indirect_write(self.id,va,&self.extents,self.count).map_err(|_|w::IO)?;
                self.phase=3;
            }
            3=>{if DiskFs::diskfs_block_sync()!=0{return Err(w::IO)}self.phase=4;}
            4=>{
                v4_cache_put(self.id,self.extents,self.count);
                unsafe{V4_TABLE[self.id].size_bytes=crate::document_journal::CAPACITY as u32;}
                v4_persist(va).map_err(|_|w::IO)?;
                self.phase=5;
            }
            5=>{if DiskFs::diskfs_block_sync()!=0{return Err(w::IO)}self.phase=6;return Ok(true)}
            _=>return Err(w::STALE),
        }
        Ok(false)
    }
}
fn document_cancel(caller:u32) {
 unsafe { if DOCUMENT_JOB.as_ref().map_or(false,|j|j.caller==caller) {
  if let Some(g)=DOCUMENT_JOB.as_ref().and_then(|j|j.growth.as_ref()){g.cancel();}
  DOCUMENT_JOB=None; DOCUMENT_WRITE_LOCK.store(0,Ordering::Relaxed);
  sex_pdx::serial_record!("[files.document.progress.retired] target={} in_flight=0 rollback=0",caller);
 } }
}
fn document_cancel_load(caller:u32){
 unsafe{if DOCUMENT_LOAD_JOB.as_ref().map_or(false,|j|j.caller==caller){
  DOCUMENT_LOAD_JOB=None;DOCUMENT_WRITE_LOCK.store(0,Ordering::Relaxed);
  sex_pdx::serial_record!("[files.document.load.retired] target={} in_flight=0",caller);
 }}
}
pub struct DocumentBackend;
// A resolved entry is one exact extent, never authority for its neighbor.
// Bound each synchronous transport to the existing permanent DMA page.
fn document_piece_len(piece:DiskManifestEntryV1,rel:u64,remaining:usize)->Result<usize,u32>{
 let available=(piece.len_bytes as u64).checked_sub(rel).ok_or(sex_pdx::document::IO)?;
 let n=remaining.min(available as usize).min(sex_pdx::BLOCK_MAX_XFER as usize);
 if n==0{return Err(sex_pdx::document::IO)}Ok(n)
}
fn document_read(id:u8,offset:u64,out:&mut[u8])->Result<(),u32>{
 use sex_pdx::document as w;let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}v4_ensure(va).map_err(|_|w::IO)?;let entry=v4_get(id as u64).map_err(|_|w::STALE)?;
 if offset.checked_add(out.len()as u64).map_or(true,|e|e>entry.size_bytes as u64){return Err(w::IO)}
 let mut done=0;while done<out.len(){let at=offset+done as u64;let(piece,rel)=v4_resolve_offset(id as usize,at,va).map_err(|_|w::IO)?;let n=document_piece_len(piece,rel,out.len()-done)?;let got=DiskFs::diskfs_read_object_entry(piece,rel,&mut out[done..done+n],va).map_err(|_|w::IO)?;if got as usize!=n{return Err(w::IO)}done+=n;}Ok(())
}
impl crate::document::Backend for DocumentBackend{
 fn publish_load(&mut self,caller:u32,r:sex_pdx::document::Request,generation:u32,name:[u8;16],bytes:&[u8])->Result<(),u32>{
  let mut region=sex_pdx::document_snapshot_view(caller,true)?;
  region.publish(sex_pdx::document_snapshot::Identity{subject:caller,object:r.object,generation,sequence:r.seq},name,bytes)?;
  sex_pdx::serial_record!("[files.document.snapshot] caller={} object={} identity={} sequence={} bytes={} published=1",caller,r.object,generation,r.seq,bytes.len());
  Ok(())
 }
 fn clear_load(&mut self,caller:u32)->Result<(),u32>{
  if !matches!(caller,9|16|18){return Ok(())}
  sex_pdx::document_snapshot_view(caller,true)?.clear()?;
  sex_pdx::serial_record!("[files.document.snapshot.clear] caller={} bytes=16384 scrubbed=1",caller);
  Ok(())
 }
 fn identity(&mut self,id:u8)->Result<u32,u32>{use sex_pdx::document as w;if !(2..15).contains(&id){return Err(w::DENIED)}let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}v4_ensure(va).map_err(|_|w::IO)?;let e=v4_get(id as u64).map_err(|_|w::STALE)?;Ok(((id as u32+1)<<16)|e.gen as u32)}
 fn name(&mut self,id:u8)->Result<[u8;16],u32>{let e=v4_get(id as u64).map_err(|_|sex_pdx::document::STALE)?;let mut name=[0;16];name.copy_from_slice(&e.name[..16]);Ok(name)}
 fn create(&mut self,name:[u8;16])->Result<u8,u32>{use sex_pdx::document as w;let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}v4_ensure(va).map_err(|_|w::IO)?;let id=handle_diskfs_create(u64::from_le_bytes(name[..8].try_into().unwrap()),u64::from_le_bytes(name[8..].try_into().unwrap()),va);if !(3..15).contains(&id){return Err(w::IO)}Ok(id as u8)}
 fn cancel_save(&mut self,caller:u32){document_cancel(caller)}
 fn cancel_load(&mut self,caller:u32){document_cancel_load(caller)}
 fn load_step(&mut self,caller:u32,id:u8,out:&mut[u8;sex_pdx::document::MAX])->Result<Option<usize>,u32>{
  use sex_pdx::document as w;
  let identity=<Self as crate::document::Backend>::identity(self,id)?;
  unsafe{
   if DOCUMENT_JOB.is_some(){return Err(w::BUSY)}
   if let Some(job)=DOCUMENT_LOAD_JOB.as_ref(){
    if(job.caller,job.object,job.identity)!=(caller,id,identity){return Err(w::BUSY)}
   }else{
    let entry=v4_get(id as u64).map_err(|_|w::STALE)?;
    DOCUMENT_LOAD_JOB=Some(DocumentLoadJob{caller,object:id,identity,
     journal:crate::document_journal::Load::new(id,identity,entry.size_bytes as u64==crate::document_journal::CAPACITY)});
    DOCUMENT_WRITE_LOCK.store(id as u64+1,Ordering::Relaxed);
    sex_pdx::serial_record!("[files.document.load.begin] caller={} object={} identity={} cooperative=1",caller,id,identity);
   }
   match DOCUMENT_LOAD_JOB.as_mut().ok_or(w::IO)?.journal.step(&mut DocumentIo{id},out)?{
    None=>Ok(None),
    Some(crate::document_journal::Loaded::Journal(r))=>{
     sex_pdx::serial_record!("[files.document.recovered] object={} identity={} generation={} bank={} bytes={} hash={:#x}",id,identity,r.generation,r.bank,r.len,r.hash);
     Ok(Some(r.len))
    },
    Some(crate::document_journal::Loaded::Legacy(len))=>Ok(Some(len)),
   }
  }
 }
 fn save_step(&mut self,caller:u32,id:u8,bytes:&[u8])->Result<bool,u32>{
  use sex_pdx::document as w;
  let identity=<Self as crate::document::Backend>::identity(self,id)?;
  unsafe {
   if DOCUMENT_LOAD_JOB.is_some(){return Err(w::BUSY)}
   if let Some(job)=DOCUMENT_JOB.as_ref(){
    if (job.caller,job.object,job.identity)!=(caller,id,identity){return Err(w::BUSY)}
   }else{
    let entry=v4_get(id as u64).map_err(|_|w::STALE)?;
    let growth=if (entry.size_bytes as u64)<crate::document_journal::CAPACITY{
        Some(DocumentGrowth::new(id as usize,diskfs_bridge_get_buf_va())?)
    }else{None};
    DOCUMENT_JOB=Some(DocumentJob{caller,object:id,identity,journal:crate::document_journal::Save::new(id,identity),growth});
    DOCUMENT_WRITE_LOCK.store(id as u64+1,Ordering::Relaxed);
    sex_pdx::serial_record!("[files.document.progress.begin] target={} object={} identity={} cooperative=1",caller,id,identity);
   }
  }
  // Reserve one run, then zero one block per turn. Metadata publication
  // is ordered after the zero barrier, before the larger visible size.
  unsafe {
   let job=DOCUMENT_JOB.as_mut().ok_or(w::IO)?;
   if let Some(g)=job.growth.as_mut(){
    if g.step(diskfs_bridge_get_buf_va())?{job.growth=None;}
    return Ok(false)
   }
  }
  if v4_get(id as u64).map_err(|_|w::STALE)?.size_bytes as u64!=crate::document_journal::CAPACITY{return Err(w::INVALID)}
  let result=unsafe{DOCUMENT_JOB.as_mut().ok_or(w::IO)?.journal.step(&mut DocumentIo{id},bytes)}?;
  if let Some(r)=result{
   sex_pdx::serial_record!("[files.document.durable] caller={} object={} identity={} generation={} bank={} bytes={} hash={:#x} barriers=3 readback=1",caller,id,identity,r.generation,r.bank,r.len,r.hash);
   Ok(true)
  }else{Ok(false)}
 }

 fn load(&mut self,id:u8,out:&mut[u8;sex_pdx::document::MAX])->Result<usize,u32>{
  use sex_pdx::document as w;let identity=<Self as crate::document::Backend>::identity(self,id)?;let e=v4_get(id as u64).map_err(|_|w::STALE)?;
  if e.size_bytes as u64==crate::document_journal::CAPACITY{
   if let Some(r)=crate::document_journal::recover(&mut DocumentIo{id},id,identity,out)?{
    sex_pdx::serial_record!("[files.document.recovered] object={} identity={} generation={} bank={} bytes={} hash={:#x}",id,identity,r.generation,r.bank,r.len,r.hash);return Ok(r.len)
   }
  }
  let mut header=[0;8];document_read(id,0,&mut header)?;if &header[..4]!=b"QP01"{return Err(w::INVALID)}let len=u32::from_le_bytes(header[4..].try_into().unwrap())as usize;if len>w::MAX{return Err(w::INVALID)}document_read(id,16,&mut out[..len])?;Ok(len)
 }

}

struct DocumentIo{id:u8}
impl crate::document_journal::Io for DocumentIo{
 fn read(&mut self,offset:u64,out:&mut[u8])->Result<(),u32>{document_read(self.id,offset,out)}
 fn write(&mut self,offset:u64,bytes:&[u8])->Result<(),u32>{
  use sex_pdx::document as w;let va=diskfs_bridge_get_buf_va();if va==0||va==u64::MAX{return Err(w::IO)}let e=v4_get(self.id as u64).map_err(|_|w::STALE)?;if offset.checked_add(bytes.len()as u64).map_or(true,|n|n>e.size_bytes as u64){return Err(w::INVALID)}
  let mut done=0;while done<bytes.len(){let at=offset+done as u64;let(piece,rel)=v4_resolve_offset(self.id as usize,at,va).map_err(|_|w::IO)?;let n=document_piece_len(piece,rel,bytes.len()-done)?;let written=DiskFs::diskfs_write_object_entry(piece,rel,&bytes[done..done+n],va).map_err(|_|w::IO)?;if written as usize!=n{return Err(w::IO)}done+=n;}Ok(())
 }
 fn sync(&mut self)->Result<(),u32>{if DiskFs::diskfs_block_sync()!=0{Err(sex_pdx::document::IO)}else{Ok(())}}
}
