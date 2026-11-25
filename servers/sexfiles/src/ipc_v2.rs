// SPDX-License-Identifier: GPL-3.0-only
//! IPC v2 endpoint lifecycle and request/reply dispatch for `sexfiles` (`sys 105`).
//!
//! `sexfiles` creates an IPC v2 endpoint at startup so callers holding a badged
//! `RIGHT_CALL` capability can invoke file-server opcodes over rendezvous IPC v2
//! (`OP_CALL` / `OP_REPLY_RECV`), while legacy v1 callers continue to be served
//! via `pdx_reply_recv` (103) / `pdx_listen_raw`.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sex_pdx::endpoint::{
    self, encode_badge, encode_word_reply, EndpointRequest, Received, E_BADMSG, E_CLOSED,
    RIGHT_CALL,
};

const NO_SLOT: u32 = u32::MAX;

static ENDPOINT_SLOT: AtomicU32 = AtomicU32::new(NO_SLOT);
static REQUESTS_SERVED: AtomicU64 = AtomicU64::new(0);
static BAD_MESSAGES: AtomicU64 = AtomicU64::new(0);

/// Result of dispatching a single IPC v2 endpoint message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchResult {
    pub label: u32,
    pub reply_words: [u64; 4],
    pub reply_word: u64,
    pub should_reply: bool,
}

/// Initializes `sexfiles`'s IPC v2 endpoint via `SYS_ENDPOINT` (`OP_CREATE`).
/// Returns `Some(slot)` on success, or `None` if endpoints are unavailable.
pub fn init_endpoint() -> Option<u32> {
    match endpoint::create_endpoint() {
        Ok(slot) => {
            ENDPOINT_SLOT.store(slot, Ordering::Release);
            sex_pdx::serial_record!("[sexfiles.ipc_v2.endpoint] slot={} ok=1", slot);
            Some(slot)
        }
        Err(err) => {
            sex_pdx::serial_record!("[sexfiles.ipc_v2.endpoint] slot=0 ok=0 err={:#x}", err);
            None
        }
    }
}

/// Returns the active IPC v2 endpoint slot if created.
#[inline]
pub fn endpoint_slot() -> Option<u32> {
    let slot = ENDPOINT_SLOT.load(Ordering::Acquire);
    if slot == NO_SLOT { None } else { Some(slot) }
}

/// Grants a badged, call-only capability to `sexfiles`'s IPC v2 endpoint into
/// the domain identified by `target_domain_slot`.
pub fn grant_to_domain(
    target_domain_slot: u32,
    caller_pd: u32,
    caller_task: u32,
) -> Result<u32, u64> {
    let slot = endpoint_slot().ok_or(E_CLOSED)?;
    let badge = encode_badge(caller_pd, caller_task);
    endpoint::grant(slot, target_domain_slot, RIGHT_CALL, badge)
}

/// Diagnostic counters `(requests_served, bad_messages)`.
#[allow(dead_code)]
pub fn stats() -> (u64, u64) {
    (
        REQUESTS_SERVED.load(Ordering::Relaxed),
        BAD_MESSAGES.load(Ordering::Relaxed),
    )
}

/// Pure IPC v2 message decoder + dispatcher: decodes `got`, invokes `handle(req)`,
/// and encodes the reply register words.
pub fn dispatch_message<H>(got: &Received, mut handle: H) -> DispatchResult
where
    H: FnMut(EndpointRequest) -> Option<u64>,
{
    let req = match EndpointRequest::decode(got) {
        Ok(r) => r,
        Err(_) => {
            BAD_MESSAGES.fetch_add(1, Ordering::Relaxed);
            let words = encode_word_reply(0, E_BADMSG).unwrap_or([0, E_BADMSG, 0, 0]);
            return DispatchResult {
                label: 0,
                reply_words: words,
                reply_word: E_BADMSG,
                should_reply: true,
            };
        }
    };
    REQUESTS_SERVED.fetch_add(1, Ordering::Relaxed);
    match handle(req) {
        Some(word) => {
            let words = encode_word_reply(req.label, word).unwrap_or([0, word, 0, 0]);
            DispatchResult {
                label: req.label,
                reply_words: words,
                reply_word: word,
                should_reply: true,
            }
        }
        None => DispatchResult {
            label: req.label,
            reply_words: [0; 4],
            reply_word: 0,
            should_reply: false,
        },
    }
}

/// Serves an IPC v2 endpoint using `OP_RECV` followed by `OP_REPLY_RECV`
/// (single kernel entry per reply+next-receive round trip).
#[allow(dead_code)]
pub fn serve_endpoint_loop<H>(slot: u32, mut handle: H) -> Result<(), u64>
where
    H: FnMut(EndpointRequest) -> Option<u64>,
{
    let mut got = endpoint::recv(slot)?;
    loop {
        let res = dispatch_message(&got, &mut handle);
        got = if res.should_reply {
            endpoint::reply_word_recv(slot, res.label, res.reply_word)?
        } else {
            endpoint::recv(slot)?
        };
    }
}
