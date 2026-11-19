// Bounded private Files RPC. Exact retries retain the sequence and copied body.
use super::super::{is_answer, is_shell, mailbox, mine, region, Turn, BUSY, FILES, SEQ};
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::{fs as f, restricted_job as w};
static STALE: AtomicU64 = AtomicU64::new(0);
struct Pending {
    epoch: u64,
    armed: bool,
}
impl Pending {
    fn abandon(&mut self) {
        if self.armed && !epoch().is_ok_and(|current| current != self.epoch) {
            STALE.store(self.epoch, Ordering::Release);
        }
        self.armed = false;
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.abandon();
    }
}
// A timed-out same-epoch provider may still own an unconsumed region reply.
// Never reuse its shared bytes until that provider incarnation is gone.
pub(crate) fn region_ready() -> Result<(), u8> {
    let old = STALE.load(Ordering::Acquire);
    if old != 0 {
        if epoch()? == old {
            return Err(f::UNAVAILABLE);
        }
        STALE.store(0, Ordering::Release);
    }
    Ok(())
}

pub fn epoch() -> Result<u64, u8> {
    let peer = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_STORAGE).map_err(|_| f::UNAVAILABLE)?;
    if peer.live() && peer.epoch != 0 {
        Ok(peer.epoch)
    } else {
        Err(f::UNAVAILABLE)
    }
}

fn call<R>(
    kind: u8,
    path: &[u8],
    action: u64,
    length: u64,
    input: &[u8],
    expected: u64,
    take: impl FnOnce(u32, &[u8]) -> Result<R, u8>,
) -> Result<R, u8> {
    if expected == 0
        || input.len() > f::DATA_MAX
        || path.len() > w::PATH_MAX
        || epoch()? != expected
    {
        return Err(f::UNAVAILABLE);
    }
    let until = sex_pdx::get_ticks()
        .checked_add(120_000)
        .ok_or(f::UNAVAILABLE)?;
    let _turn = loop {
        if sex_pdx::get_ticks() >= until || epoch()? != expected {
            return Err(f::UNAVAILABLE);
        }
        if BUSY
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            break Turn;
        }
        sex_pdx::sys_yield();
    };
    let area = region()?;
    area[..path.len()].copy_from_slice(path);
    area[f::PATH_BYTES..f::PATH_BYTES + input.len()].copy_from_slice(input);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & 0xff_ffff;
    let request = f::Request {
        seq,
        kind,
        path_len: path.len() as u16,
        offset: action,
        length,
    }
    .args();
    let mut sent = false;
    let mut pending = Pending {
        epoch: expected,
        armed: false,
    };
    loop {
        let now = sex_pdx::get_ticks();
        if epoch()? != expected {
            return Err(f::UNAVAILABLE);
        }
        if now >= until {
            return Err(f::UNAVAILABLE);
        }
        // The admitted kernel queue is reliable. Retries after successful
        // enqueue could rearm a cached shared-region reply after consumption.
        if !sent {
            if sex_pdx::pdx_call(
                sex_pdx::SLOT_STORAGE,
                f::OP,
                request[0],
                request[1],
                request[2],
            )
            .0 == 0
            {
                sent = true;
                pending.armed = true;
            }
        }
        let message = mailbox::take(|m| mine(m, seq) && m.caller_task == FILES)
            .or_else(|| sex_pdx::pdx_try_listen_raw(0));
        if let Some(m) = message {
            if mine(&m, seq) && m.caller_task == FILES {
                if !sent || epoch()? != expected {
                    return Err(f::UNAVAILABLE);
                }
                let (_, status, value) = f::decode_reply(m.arg0).ok_or(f::INVALID)?;
                pending.armed = false;
                if status != f::OK {
                    return Err(status);
                }
                let result = take(value, &area[f::PATH_BYTES..f::CACHE_AT]);
                if epoch()? != expected {
                    return Err(f::UNAVAILABLE);
                }
                return result;
            }
            if is_answer(&m) || is_shell(&m) {
                mailbox::put(m);
            }
        } else {
            if sex_pdx::sys_wait_self_ticks(1).is_err() {
                sex_pdx::sys_yield();
            }
        }
    }
}
fn rpc(action: u64, input: &[u8], expected: u64) -> Result<(u32, [u8; w::INFO_BYTES]), u8> {
    if input.len() > w::MAX {
        return Err(f::INVALID);
    }
    call(
        w::KIND,
        b"",
        action,
        input.len() as u64,
        input,
        expected,
        |value, data| {
            let mut bytes = [0; w::INFO_BYTES];
            bytes.copy_from_slice(&data[..w::INFO_BYTES]);
            Ok((value, bytes))
        },
    )
}

/// Ordinary file operations under the job's held Files epoch and bounded turn.
pub fn stat(path: &[u8], expected: u64) -> Result<(bool, u64), u8> {
    call(f::STAT, path, 0, 0, &[], expected, |n, data| {
        if n != 32 {
            return Err(f::INVALID);
        }
        let kind = u64::from_le_bytes(data[..8].try_into().unwrap());
        if kind > 1 {
            return Err(f::INVALID);
        }
        Ok((
            kind != 0,
            u64::from_le_bytes(data[8..16].try_into().unwrap()),
        ))
    })
}
pub fn create(path: &[u8], expected: u64) -> Result<(), u8> {
    call(f::CREATE, path, 0, 0, &[], expected, |_, _| Ok(()))
}
pub fn sync(expected: u64) -> Result<(), u8> {
    call(f::SYNC, b"", 0, 0, &[], expected, |_, _| Ok(()))
}
pub fn write_at(path: &[u8], offset: u64, data: &[u8], expected: u64) -> Result<usize, u8> {
    let n = data.len().min(f::DATA_MAX);
    call(
        f::WRITE,
        path,
        offset,
        n as u64,
        &data[..n],
        expected,
        |wrote, _| {
            if wrote as usize > n {
                Err(f::INVALID)
            } else {
                Ok(wrote as usize)
            }
        },
    )
}
pub fn read_at(path: &[u8], offset: u64, out: &mut [u8], expected: u64) -> Result<usize, u8> {
    let want = out.len().min(f::DATA_MAX);
    call(
        f::READ,
        path,
        offset,
        want as u64,
        &[],
        expected,
        |n, data| {
            let n = n as usize;
            if n > want {
                return Err(f::INVALID);
            }
            out[..n].copy_from_slice(&data[..n]);
            Ok(n)
        },
    )
}

pub fn prepare(request: &w::Prepare<'_>) -> Result<w::Info, u8> {
    let expected = epoch()?;
    let mut body = [0; w::MAX];
    let n = request.encode(&mut body).ok_or(f::INVALID)?;
    let (ticket, bytes) = rpc(w::PREPARE, &body[..n], expected)?;
    let info = w::Info::decode(&bytes).ok_or(f::INVALID)?;
    if info.ticket != ticket
        || info.files_epoch != expected
        || info.nonce != request.nonce
        || info.cookie != request.cookie
        || info.format != request.format as u32
        || info.child.is_some()
        || info.output_hash != [0; 32]
    {
        return Err(f::INVALID);
    }
    Ok(info)
}

pub fn result(ticket: u32, nonce: [u8; 16], expected: u64) -> Result<w::Info, u8> {
    if ticket == 0 || ticket > 0xff_ffff || nonce == [0; 16] {
        return Err(f::INVALID);
    }
    let (n, bytes) = rpc((ticket as u64) << 8 | w::RESULT, &nonce, expected)?;
    if n as usize != w::INFO_BYTES {
        return Err(f::INVALID);
    }
    let info = w::Info::decode(&bytes).ok_or(f::INVALID)?;
    if info.ticket != ticket || info.nonce != nonce || info.files_epoch != expected {
        return Err(f::INVALID);
    }
    Ok(info)
}

fn end(ticket: u32, nonce: [u8; 16], expected: u64, action: u64) -> Result<(), u8> {
    if ticket == 0 || ticket > 0xff_ffff || nonce == [0; 16] {
        return Err(f::INVALID);
    }
    let (n, _) = rpc((ticket as u64) << 8 | action, &nonce, expected)?;
    if n != 0 {
        return Err(f::INVALID);
    }
    Ok(())
}
pub fn cancel(ticket: u32, nonce: [u8; 16], expected: u64) -> Result<(), u8> {
    end(ticket, nonce, expected, w::CANCEL)
}
pub fn release(ticket: u32, nonce: [u8; 16], expected: u64) -> Result<(), u8> {
    end(ticket, nonce, expected, w::RELEASE)
}

/// Nonblocking cleanup. Keep polling and retain the object until completion;
/// dropping an admitted, unacknowledged request poisons same-epoch region reuse.
pub struct End {
    // Declaration order also protects abandonment: poison before unlocking.
    pending: Pending,
    turn: Option<Turn>,
    seq: u32,
    request: [u64; 3],
    until: u64,
    sent: bool,
    done: bool,
}
pub fn begin_end(
    ticket: u32,
    nonce: [u8; 16],
    expected: u64,
    release: bool,
) -> Result<Option<End>, u8> {
    if ticket == 0 || ticket > 0xff_ffff || nonce == [0; 16] {
        return Err(f::INVALID);
    }
    region_ready()?;
    if expected == 0 || epoch()? != expected {
        return Err(f::UNAVAILABLE);
    }
    let until = sex_pdx::get_ticks()
        .checked_add(120_000)
        .ok_or(f::UNAVAILABLE)?;
    if BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Ok(None);
    }
    let turn = Turn;
    let area = region()?;
    area[f::PATH_BYTES..f::PATH_BYTES + 16].copy_from_slice(&nonce);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & 0xff_ffff;
    let action = if release { w::RELEASE } else { w::CANCEL };
    let request = f::Request {
        seq,
        kind: w::KIND,
        path_len: 0,
        offset: (ticket as u64) << 8 | action,
        length: 16,
    }
    .args();
    Ok(Some(End {
        pending: Pending {
            epoch: expected,
            armed: false,
        },
        turn: Some(turn),
        seq,
        request,
        until,
        sent: false,
        done: false,
    }))
}
impl End {
    fn finish(&mut self, result: Result<(), u8>) -> Option<Result<(), u8>> {
        self.pending.abandon();
        self.turn.take();
        self.done = true;
        Some(result)
    }
    /// Performs at most one enqueue and one inbox intake per call; never waits.
    pub fn poll(&mut self) -> Option<Result<(), u8>> {
        if self.done {
            return None;
        }
        if epoch().ok() != Some(self.pending.epoch) || sex_pdx::get_ticks() >= self.until {
            return self.finish(Err(f::UNAVAILABLE));
        }
        if !self.sent {
            if sex_pdx::pdx_call(
                sex_pdx::SLOT_STORAGE,
                f::OP,
                self.request[0],
                self.request[1],
                self.request[2],
            )
            .0 == 0
            {
                self.sent = true;
                self.pending.armed = true;
            }
        }
        let message = mailbox::take(|m| mine(m, self.seq) && m.caller_task == FILES)
            .or_else(|| sex_pdx::pdx_try_listen_raw(0));
        if let Some(m) = message {
            if mine(&m, self.seq) && m.caller_task == FILES {
                if !self.sent || epoch().ok() != Some(self.pending.epoch) {
                    return self.finish(Err(f::UNAVAILABLE));
                }
                let (_, status, n) = f::decode_reply(m.arg0).unwrap();
                self.pending.armed = false;
                return self.finish(if status != f::OK {
                    Err(status)
                } else if n != 0 {
                    Err(f::INVALID)
                } else {
                    Ok(())
                });
            }
            if is_answer(&m) || is_shell(&m) {
                mailbox::put(m);
            }
        }
        None
    }
}
