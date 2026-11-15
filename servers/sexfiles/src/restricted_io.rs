//! Scoped parent job transport. It consumes only the new kind, ahead of generic
//! payload logs. No ordinary PREPARE ticket, mutable source path or client digest
//! is accepted as launch authority. Child positive I/O is wired separately.
extern crate alloc;
use crate::restricted_job::Error;
use crate::restricted_jobs::Jobs;
use crate::restricted_storage::Storage;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::restricted_helper::Binding;
use sex_pdx::restricted_helper::Subject;
use sex_pdx::{app_catalog as catalog, fs as f, restricted_job as wire};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
static mut JOBS: Jobs = Jobs::new();
static mut LOAD: Option<crate::restricted_load::Load> = None;
static mut CONTROL: Option<crate::restricted_control::Control> = None;
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
    data: [u8; 176],
    data_len: usize,
    scope: Option<(u32, [u8; 16])>,
}
// One request in flight per catalog Program. A foreign caller cannot consume a
// legitimate parent's reply slot. Last complete request is retained for exact
// retry; altered same-sequence retries never repeat a side effect.
struct ChildReply {
    actor: Subject,
    binding: Binding,
    epoch: u64,
    request: [u64; 3],
    fingerprint: [u8; 32],
    data: Zeroizing<Vec<u8>>,
    word: u64,
    waiting: bool,
}
static mut CHILD: [Option<ChildReply>; 2] = [None, None];
static mut REPLIES: [Option<Reply>; 16] = [None; 16];
fn clear(slot: &mut Option<Reply>) {
    if let Some(reply) = slot.as_mut() {
        for byte in &mut reply.input {
            unsafe { core::ptr::write_volatile(byte, 0) }
        }
        for byte in &mut reply.data {
            unsafe { core::ptr::write_volatile(byte, 0) }
        }
        if let Some((id, nonce)) = reply.scope.as_mut() {
            *id = 0;
            for byte in nonce {
                unsafe { core::ptr::write_volatile(byte, 0) }
            }
        }
        reply.scope = None;
        reply.data_len = 0;
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
fn control(opcode: u64, actor: Subject, words: [u64; 3]) -> bool {
    if !matches!(
        opcode,
        wire::OP_CLAIM | wire::OP_DESCRIBE | wire::OP_LOAD | wire::OP_COPIED
    ) {
        return false;
    }
    if actor != (Subject { pd: 3, task: 3 }) {
        return true;
    }
    let Some(own_epoch) = epoch() else {
        return true;
    };
    let now = sex_pdx::get_ticks();
    let current = unsafe { &mut *core::ptr::addr_of_mut!(CONTROL) };
    let mut turn = Turn::enter();
    if let Some(c) = current.as_ref() {
        if !c.valid(own_epoch, now)
            || turn
                .jobs()
                .get(c.ticket)
                .is_none_or(|e| !e.live(now, own_epoch) || !live(e.job.parent()))
        {
            *current = None;
        }
    }
    let loaded = unsafe { &mut *core::ptr::addr_of_mut!(LOAD) };
    if let Some(l) = loaded.as_ref() {
        if !l.valid(own_epoch, now)
            || turn
                .jobs()
                .get(l.ticket)
                .is_none_or(|e| !e.live(now, own_epoch) || !live(e.job.parent()))
        {
            *loaded = None;
        }
    }
    if opcode == wire::OP_LOAD {
        if let Some(l) = loaded.as_mut() {
            l.repeat(words);
            return true;
        }
        let Some(c) = current
            .as_ref()
            .filter(|c| c.complete() && c.ticket as u64 == words[0])
        else {
            return true;
        };
        let Some(scope) = crate::restricted_runtime::staging()
            .filter(|b| b.generation == words[1] && b.previous.word() == words[2])
        else {
            return true;
        };
        let Some(e) = turn
            .jobs()
            .get(c.ticket)
            .filter(|e| e.job.matches_scope(scope))
        else {
            return true;
        };
        if !e.live(now, own_epoch) || !live(e.job.parent()) {
            return true;
        }
        let Some((lease, length)) = crate::restricted_load::image(scope) else {
            return true;
        };
        if epoch() != Some(own_epoch)
            || crate::restricted_runtime::staging() != Some(scope)
            || !live(e.job.parent())
        {
            return true;
        }
        *loaded =
            crate::restricted_load::Load::new(c.ticket, scope, lease, length, sex_pdx::get_ticks());
        return true;
    }
    if opcode == wire::OP_COPIED {
        if let Some(l) = loaded.as_mut() {
            if crate::restricted_runtime::staging() == Some(l.scope) {
                l.copied(words, |lease| {
                    crate::restricted_image::copied(lease, lease.shell_epoch, own_epoch)
                });
            }
        }
        return true;
    }
    if opcode == wire::OP_DESCRIBE && words[1] == 22 {
        if let Some(l) = loaded.as_mut() {
            l.serial(words);
        }
        return true;
    }
    if opcode == wire::OP_DESCRIBE {
        if let Some(c) = current.as_mut() {
            c.fetch(words, now);
        }
        return true;
    }
    if let Some(c) = current.as_mut() {
        if c.repeat_claim(words, own_epoch, now) {
            return true;
        }
        if !c.complete() {
            return true;
        }
    }
    let Some(parent) = Subject::decode(words[1]) else {
        return true;
    };
    let Ok(cookie) = u32::try_from(words[2]) else {
        return true;
    };
    let Ok(ticket) = u32::try_from(words[0]) else {
        return true;
    };
    if !live(parent) {
        return true;
    }
    let Some(e) = turn.jobs().get(ticket) else {
        return true;
    };
    if e.claim(parent, cookie, now, own_epoch).is_err() {
        return true;
    }
    if let Ok(info) = e.info(false) {
        *current = crate::restricted_control::Control::new(words, own_epoch, info, now);
    }
    true
}
fn poll_control() {
    let Some(own_epoch) = epoch() else { return };
    let now = sex_pdx::get_ticks();
    let current = unsafe { &mut *core::ptr::addr_of_mut!(CONTROL) };
    let Some(c) = current.as_mut() else { return };
    let mut turn = Turn::enter();
    if !c.valid(own_epoch, now)
        || turn
            .jobs()
            .get(c.ticket)
            .is_none_or(|e| !e.live(now, own_epoch) || !live(e.job.parent()))
    {
        *current = None;
        return;
    }
    if let Some(l) = unsafe { &mut *core::ptr::addr_of_mut!(LOAD) } {
        if !l.valid(own_epoch, now)
            || turn
                .jobs()
                .get(l.ticket)
                .is_none_or(|e| !e.live(now, own_epoch) || !live(e.job.parent()))
        {
            unsafe {
                *core::ptr::addr_of_mut!(LOAD) = None;
            }
            return;
        }
        if let Some(word) = l.reply() {
            if sex_pdx::pdx_reply(3, word) == 0 {
                l.delivered();
            }
        }
    }
    if let Some(word) = c.reply() {
        if sex_pdx::pdx_reply(3, word) == 0 {
            c.delivered();
        }
    }
}
pub fn expire(binding: Binding) -> bool {
    Turn::enter().jobs().expire(binding);
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(CHILD) } {
        if slot.as_ref().is_some_and(|r| r.binding == binding) {
            *slot = None;
        }
    }
    true
}
pub fn retire(actor: Subject) -> bool {
    {
        let mut turn = Turn::enter();
        turn.jobs().retire(actor);
        for slot in unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) } {
            if slot.as_ref().is_some_and(|r| r.actor == actor) {
                clear(slot);
            }
        }
        for slot in unsafe { &mut *core::ptr::addr_of_mut!(CHILD) } {
            if slot
                .as_ref()
                .is_some_and(|r| r.actor == actor || r.binding.parent == actor)
            {
                *slot = None;
            }
        }
        let current = unsafe { &mut *core::ptr::addr_of_mut!(CONTROL) };
        if current
            .as_ref()
            .is_some_and(|c| turn.jobs().get(c.ticket).is_none())
        {
            *current = None;
            unsafe {
                *core::ptr::addr_of_mut!(LOAD) = None;
            }
        }
    }
    !crate::restricted_runtime::denied(actor.pd) || crate::restricted_runtime::scrub_retired(actor)
}
pub fn install(binding: Binding) -> bool {
    let Some(own_epoch) = epoch() else {
        return false;
    };
    let now = sex_pdx::get_ticks();
    let Some(l) = unsafe { &*core::ptr::addr_of!(LOAD) }
        .as_ref()
        .filter(|l| l.valid(own_epoch, now) && l.install(binding))
    else {
        return false;
    };
    if !live(binding.parent) {
        return false;
    }
    let mut turn = Turn::enter();
    let Some(e) = turn.jobs().get(l.ticket) else {
        return false;
    };
    e.bind(binding, now, own_epoch).is_ok()
}
fn copy_grant(address: u64, at: usize, n: usize) -> Result<Zeroizing<Vec<u8>>, Error> {
    if n > crate::restricted_job::MAX_TRANSFER {
        return Err(Error::Bounds);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.try_reserve_exact(n).map_err(|_| Error::Memory)?;
    bytes.resize(n, 0);
    for i in 0..n {
        bytes[i] = unsafe { core::ptr::read_volatile((address as usize + at + i) as *const u8) }
    }
    Ok(bytes)
}
fn child_message(actor: Subject, r: f::Request, words: [u64; 3]) -> bool {
    let deny = || {
        sex_pdx::pdx_reply(actor.pd, f::reply(r.seq, f::DENIED, 0));
    };
    let Some(own_epoch) = epoch() else {
        return true;
    };
    if !live(actor) {
        return true;
    }
    let Some(address) = view(actor) else {
        return true;
    };
    if r.path_len as usize > 192 || r.length > crate::restricted_job::MAX_TRANSFER as u64 {
        deny();
        return true;
    }
    let mut path = [0; 192];
    for i in 0..r.path_len as usize {
        path[i] = unsafe { core::ptr::read_volatile((address as usize + i) as *const u8) }
    }
    let input = if r.kind == f::WRITE {
        match copy_grant(address, f::PATH_BYTES, r.length as usize) {
            Ok(v) => v,
            Err(_) => {
                deny();
                return true;
            }
        }
    } else {
        Zeroizing::new(Vec::new())
    };
    let mut hasher = Sha256::new();
    hasher.update(&path[..r.path_len as usize]);
    hasher.update(input.as_slice());
    let fingerprint: [u8; 32] = hasher.finalize().into();
    let replies = unsafe { &mut *core::ptr::addr_of_mut!(CHILD) };
    if let Some(previous) = replies.iter_mut().flatten().find(|p| p.actor == actor) {
        if previous.request[0] & 0xff_ffff == words[0] & 0xff_ffff {
            if previous.request == words && previous.fingerprint == fingerprint {
                previous.waiting = true;
            }
            return true;
        }
        if previous.waiting {
            return true;
        }
    }
    let slot = match replies
        .iter()
        .position(|p| p.as_ref().is_some_and(|p| p.actor == actor))
        .or_else(|| replies.iter().position(Option::is_none))
    {
        Some(i) => i,
        None => {
            deny();
            return true;
        }
    };
    let mut turn = Turn::enter();
    let Some(e) = turn.jobs().child(actor) else {
        deny();
        return true;
    };
    let Some(binding) = e.job.binding() else {
        deny();
        return true;
    };
    let now = sex_pdx::get_ticks();
    let allowed = e.live(now, own_epoch)
        && live(binding.parent)
        && crate::restricted_runtime::job_live(binding, now);
    let result = crate::restricted_data::run(
        e,
        actor,
        binding,
        allowed,
        r,
        &path[..r.path_len as usize],
        &input,
    )
    .and_then(|response| match response {
        crate::restricted_data::Reply::Publish => {
            let mtime = sex_pdx::civil_time_ms().map_or(0, |t| t / 1000);
            crate::fs_service::restricted_storage(|fs| {
                e.storage
                    .publish(fs, e.job.publication_bytes()?, mtime, || {
                        epoch() == Some(own_epoch)
                            && live(actor)
                            && live(binding.parent)
                            && crate::restricted_runtime::job_live(binding, sex_pdx::get_ticks())
                    })
            })?;
            e.job.published();
            Ok((Zeroizing::new(Vec::new()), 0))
        }
        crate::restricted_data::Reply::Data(data) => {
            let n = data.len();
            Ok((data, n))
        }
        crate::restricted_data::Reply::Count(n) => Ok((Zeroizing::new(Vec::new()), n)),
    });
    let (data, word) = match result {
        Ok((data, n)) => (data, f::reply(r.seq, f::OK, n as u32)),
        Err(error) => (
            Zeroizing::new(Vec::new()),
            f::reply(r.seq, status(error), 0),
        ),
    };
    replies[slot] = Some(ChildReply {
        actor,
        binding,
        epoch: own_epoch,
        request: words,
        fingerprint,
        data,
        word,
        waiting: true,
    });
    true
}
fn poll_children() {
    let Some(own_epoch) = epoch() else { return };
    let now = sex_pdx::get_ticks();
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(CHILD) } {
        let Some(reply) = slot.as_ref() else { continue };
        let mut turn = Turn::enter();
        if reply.epoch != own_epoch
            || !live(reply.actor)
            || !live(reply.binding.parent)
            || !crate::restricted_runtime::job_live(reply.binding, now)
            || turn
                .jobs()
                .child(reply.actor)
                .is_none_or(|e| !e.live(now, own_epoch) || e.job.binding() != Some(reply.binding))
        {
            *slot = None;
            continue;
        }
        let reply = slot.as_mut().unwrap();
        if !reply.waiting {
            continue;
        }
        let Some(address) = view(reply.actor) else {
            *slot = None;
            continue;
        };
        for (i, byte) in reply.data.iter().enumerate() {
            unsafe {
                core::ptr::write_volatile((address as usize + f::PATH_BYTES + i) as *mut u8, *byte)
            }
        }
        if epoch() != Some(own_epoch)
            || !live(reply.actor)
            || !crate::restricted_runtime::job_live(reply.binding, sex_pdx::get_ticks())
        {
            *slot = None;
            continue;
        }
        if sex_pdx::pdx_reply(reply.actor.pd, reply.word) == 0 {
            reply.waiting = false;
        }
    }
}
pub fn message(opcode: u64, actor: Subject, words: [u64; 3]) -> bool {
    if control(opcode, actor, words) {
        return true;
    }
    if opcode != f::OP {
        return false;
    }
    let Some(r) = f::Request::decode(words[0], words[1], words[2]) else {
        return false;
    };
    if crate::restricted_runtime::denied(actor.pd) {
        return child_message(actor, r, words);
    }
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
            let action = r.offset & 255;
            let id = u32::try_from(r.offset >> 8).map_err(|_| Error::Bounds)?;
            if r.length != 16 {
                return Err(Error::Header);
            }
            let nonce: [u8; 16] = input[..16].try_into().unwrap();
            if action == wire::CANCEL || action == wire::RELEASE {
                if !turn
                    .jobs()
                    .release(id, actor, nonce, sex_pdx::get_ticks(), own_epoch)
                {
                    return Err(Error::Identity);
                }
                return Ok((0, [0; 176], 0, None));
            }
            if action != wire::RESULT {
                return Err(Error::State);
            }
            let e = turn
                .jobs()
                .get(id)
                .filter(|e| e.parent(actor, nonce, sex_pdx::get_ticks(), own_epoch))
                .ok_or(Error::Identity)?;
            return Ok((176, e.info(true)?, 176, Some((id, nonce))));
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
        Ok((id, [0; 176], 0, Some((id, p.nonce))))
    })();
    let (word, data, data_len, scope) = match result {
        Ok((value, data, n, scope)) => (f::reply(r.seq, f::OK, value), data, n, scope),
        Err(e) => (f::reply(r.seq, status(e), 0), [0; 176], 0, None),
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
        data,
        data_len,
        scope,
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
    poll_control();
    poll_children();
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
        if let Some((id, nonce)) = reply.scope {
            if Turn::enter()
                .jobs()
                .get(id)
                .is_none_or(|e| !e.parent(reply.actor, nonce, sex_pdx::get_ticks(), own_epoch))
            {
                clear(slot);
                continue;
            }
        }
        if reply.waiting && reply.data_len != 0 {
            let Some(address) = view(reply.actor) else {
                clear(slot);
                continue;
            };
            for i in 0..reply.data_len {
                unsafe {
                    core::ptr::write_volatile(
                        (address as usize + f::PATH_BYTES + i) as *mut u8,
                        reply.data[i],
                    )
                }
            }
        }
        if reply.waiting && sex_pdx::pdx_reply(reply.actor.pd, reply.word) == 0 {
            reply.waiting = false;
        }
    }
}
