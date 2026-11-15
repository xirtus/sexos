//! Scoped parent job transport. It consumes only the new kind, ahead of generic
//! payload logs. No ordinary PREPARE ticket, mutable source path or client digest
//! is accepted as launch authority. Child positive I/O is wired separately.
use crate::restricted_job::Error;
use crate::restricted_jobs::Jobs;
use crate::restricted_storage::Storage;
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::restricted_helper::Subject;
use sex_pdx::{app_catalog as catalog, fs as f, restricted_job as wire};
static mut JOBS: Jobs = Jobs::new();
static ENTERED: AtomicBool = AtomicBool::new(false);
struct Turn;
impl Turn {
    fn enter() -> Self {
        assert!(
            !ENTERED.swap(true, Ordering::Acquire),
            "RESTRICTED_JOB_REENTRY"
        );
        Self
    }
    fn jobs(&mut self) -> &mut Jobs {
        unsafe { &mut *core::ptr::addr_of_mut!(JOBS) }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        ENTERED.store(false, Ordering::Release);
    }
}
#[derive(Clone, Copy)]
struct Reply {
    actor: Subject,
    epoch: u64,
    request: [u64; 3],
    input: [u8; wire::MAX],
    length: usize,
    word: u64,
    waiting: bool,
}
// One request in flight per catalog Program. A foreign caller cannot consume a
// legitimate parent's reply slot. Last complete request is retained for exact
// retry; altered same-sequence retries never repeat a side effect.
static mut REPLIES: [Option<Reply>; 16] = [None; 16];
fn clear(slot: &mut Option<Reply>) {
    if let Some(reply) = slot.as_mut() {
        for byte in &mut reply.input {
            unsafe { core::ptr::write_volatile(byte, 0) }
        }
        reply.request = [0; 3];
        reply.word = 0;
        reply.actor = Subject { pd: 0, task: 0 };
        reply.epoch = 0;
        reply.length = 0;
        reply.waiting = false;
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
    }
    *slot = None;
}
fn ordinal(pd: u32) -> Option<usize> {
    catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .position(|r| r.target == pd)
        .filter(|i| *i < 16)
}
fn epoch() -> Option<u64> {
    sex_pdx::crash_log::self_epoch().ok().filter(|e| *e != 0)
}
fn live(actor: Subject) -> bool {
    crate::fs_service::live_run(actor.pd, actor.task)
}
fn view(actor: Subject) -> Option<u64> {
    let index = f::PROGRAM_INDEX_BASE + ordinal(actor.pd)?;
    let slot = f::PROVIDER_SLOT_BASE + index as u64;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    let info = sex_pdx::memory_region_info(slot).ok()?;
    if info.peer_pd != Some(actor.pd)
        || info.region_id != f::REGION_ID_BASE + index as u32
        || info.length != f::REGION_BYTES as u64
        || info.rights & (QUERY_VIEW | READ | WRITE) != (QUERY_VIEW | READ | WRITE)
    {
        return None;
    }
    let v = sex_pdx::memory_region_view(slot, 0, info.length, QUERY_VIEW | READ | WRITE).ok()?;
    (v.region_id == info.region_id
        && v.length == info.length
        && v.virtual_address != 0
        && v.virtual_address % 4096 == 0)
        .then_some(v.virtual_address)
}
fn status(error: Error) -> u8 {
    match error {
        Error::Memory | Error::Budget => f::NO_SPACE,
        Error::Io => f::IO,
        Error::Unsupported => f::UNSUPPORTED,
        Error::Identity | Error::ReadOnly => f::DENIED,
        _ => f::INVALID,
    }
}
pub fn message(opcode: u64, actor: Subject, words: [u64; 3]) -> bool {
    if opcode != f::OP {
        return false;
    }
    let Some(r) = f::Request::decode(words[0], words[1], words[2]) else {
        return false;
    };
    if r.kind != wire::KIND {
        return false;
    }
    // Unknown/stale clients receive no reply routed to a possibly reused row.
    let Some(index) = ordinal(actor.pd) else {
        return true;
    };
    let Some(own_epoch) = epoch() else {
        return true;
    };
    if !live(actor) || crate::restricted_runtime::denied(actor.pd) {
        return true;
    }
    let Some(address) = view(actor) else {
        return true;
    };
    if r.path_len != 0 || r.length > wire::MAX as u64 {
        return true;
    }
    let mut input = [0; wire::MAX];
    for i in 0..r.length as usize {
        input[i] =
            unsafe { core::ptr::read_volatile((address as usize + f::PATH_BYTES + i) as *const u8) }
    }
    let replies = unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) };
    if let Some(previous) = replies[index].as_mut() {
        if previous.actor == actor && previous.epoch == own_epoch {
            if previous.request[0] & 0xff_ffff == words[0] & 0xff_ffff {
                if previous.request == words
                    && previous.length == r.length as usize
                    && previous.input == input
                {
                    previous.waiting = true;
                }
                return true;
            }
            if previous.waiting {
                return true;
            }
        }
    }
    let mut turn = Turn::enter();
    let result = (|| {
        if r.offset != wire::PREPARE {
            return Err(Error::State);
        }
        let p = wire::Prepare::decode(&input[..r.length as usize]).ok_or(Error::Header)?;
        let mut absolute = [[0; f::PATH_BYTES]; 3];
        let mut lengths = [0; 3];
        for i in 0..3 {
            lengths[i] = crate::fs_service::restricted_path(actor, p.paths[i], &mut absolute[i])
                .ok_or(Error::Identity)?;
        }
        let now = sex_pdx::get_ticks();
        let id = turn.jobs().prepare(own_epoch, now, p.format, |id| {
            crate::fs_service::restricted_storage(|fs| {
                Storage::capture(
                    fs,
                    p.paths,
                    [
                        &absolute[0][..lengths[0]],
                        &absolute[1][..lengths[1]],
                        &absolute[2][..lengths[2]],
                    ],
                    id,
                    actor,
                    p.cookie,
                    p.nonce,
                )
            })
        })?;
        if epoch() != Some(own_epoch) || !live(actor) {
            turn.jobs().release(id, actor, p.nonce, now, own_epoch);
            return Err(Error::Identity);
        }
        Ok(id)
    })();
    let word = match result {
        Ok(id) => f::reply(r.seq, f::OK, id),
        Err(e) => f::reply(r.seq, status(e), 0),
    };
    clear(&mut replies[index]);
    replies[index] = Some(Reply {
        actor,
        epoch: own_epoch,
        request: words,
        input,
        length: r.length as usize,
        word,
        waiting: true,
    });
    true
}
pub fn pending() -> bool {
    Turn::enter().jobs().pending()
        || unsafe {
            (*core::ptr::addr_of!(REPLIES))
                .iter()
                .flatten()
                .any(|r| r.waiting)
        }
}
pub fn poll() {
    let Some(own_epoch) = epoch() else { return };
    let now = sex_pdx::get_ticks();
    Turn::enter().jobs().sweep(now, own_epoch, live, |binding| {
        crate::restricted_runtime::job_live(binding, now)
    });
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) } {
        let Some(reply) = slot else { continue };
        if reply.epoch != own_epoch
            || !live(reply.actor)
            || crate::restricted_runtime::denied(reply.actor.pd)
        {
            clear(slot);
            continue;
        }
        if reply.waiting && sex_pdx::pdx_reply(reply.actor.pd, reply.word) == 0 {
            reply.waiting = false;
        }
    }
}
