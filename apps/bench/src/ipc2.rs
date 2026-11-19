// SPDX-License-Identifier: GPL-3.0-only
//! `run bench ipc2`: IPC v2 (syscalls 105/106) checked and timed. A worker serves an
//! endpoint with RECV / REPLY_RECV; the main task calls it. Checks badges, payloads,
//! rights, a one-way send and a notification, then times call round trips.
use sex_pdx::endpoint::{self as ep, *};

const STOP: u32 = 0xFFFF;
const BADGE: u64 = 0x5EC5;

fn tsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

/// The server: answers label L with (L, a + 1, b ^ badge, c), counts one-way sends.
fn serve(slot: u32) {
    let mut got = ep::endpoint(OP_RECV, slot, [0; 4]);
    loop {
        let Ok(m) = got else { sex_pdx::serial_record!("[bench.ipc2.server_error] {:x}", got.unwrap_err()); return };
        let info = MessageInfo::from_raw(m.words[0]).unwrap_or(MessageInfo::EMPTY);
        if info.label() == STOP { let _ = ep::endpoint(OP_REPLY, 0, [m.words[0], 0, 0, 0]); return; }
        let answer = [m.words[0], m.words[1] + 1, m.words[2] ^ m.badge, m.words[3]];
        got = ep::endpoint(OP_REPLY_RECV, slot, answer);
    }
}

pub fn run() -> Result<[(&'static str, u64, u64); 2], &'static str> {
    let root = ep::create_endpoint().map_err(|_| "create endpoint")?;
    let server = sex_app::thread::spawn(move || serve(root)).map_err(|_| "spawn server")?;
    let r = ep::call(root, 7, [41, 0, 9]).map_err(|_| "first call")?;
    if r.words[1] != 42 || r.words[3] != 9 || MessageInfo::from_raw(r.words[0]).map(|i| i.label()) != Ok(7) { return Err("wrong answer"); }
    // Unbadged root: the server sees badge 0, so b ^ 0 == b.
    if r.words[2] != 0 { return Err("unexpected badge"); }
    // A badged copy that may only call: the server sees the badge, and it cannot receive.
    let client = ep::endpoint(OP_DERIVE, root, [0, (1 << 2) | (1 << 3), BADGE, 0]).map_err(|_| "derive")?.words[0] as u32;
    let r = ep::call(client, 2, [1, 0xFF, 0]).map_err(|_| "badged call")?;
    if r.words[2] != 0xFF ^ BADGE { return Err("badge not delivered"); }
    if ep::endpoint(OP_RECV, client, [0; 4]) != Err(E_RIGHTS) { return Err("call-only copy could receive"); }
    if ep::endpoint(OP_DERIVE, client, [0, 1 << 3, BADGE + 1, 0]).is_ok() { return Err("badge changed on a badged copy"); }
    let n = 2_000u64;
    let t = tsc();
    for i in 0..n {
        let r = ep::call(root, 1, [i, BADGE, 0]).map_err(|_| "call failed")?;
        if r.words[1] != i + 1 { return Err("round trip mixed up"); }
    }
    let trip = tsc() - t;
    // Notification: signal bits, then a wait that finds them at once.
    let note = ep::notify(OP_CREATE, 0, 0).map_err(|_| "create notification")? as u32;
    ep::notify(OP_SIGNAL, note, 0b1010).map_err(|_| "signal")?;
    ep::notify(OP_SIGNAL, note, 0b0001).map_err(|_| "signal")?;
    if ep::notify(OP_NOTIFY_WAIT, note, 0) != Ok(0b1011) { return Err("notification bits"); }
    if ep::notify(OP_POLL, note, 0) != Ok(0) { return Err("bits taken twice"); }
    let t = tsc();
    for _ in 0..n { let _ = ep::notify(OP_SIGNAL, note, 1); let _ = ep::notify(OP_POLL, note, 0); }
    let signal = tsc() - t;
    // Stop the server, then the endpoint is gone for everyone.
    let stop = MessageInfo::new(STOP, 0, 0).map_err(|_| "info")?.raw();
    ep::endpoint(OP_CALL, root, [stop, 0, 0, 0]).map_err(|_| "stop call")?;
    server.join();
    ep::endpoint(OP_DESTROY, root, [0; 4]).map_err(|_| "destroy")?;
    if ep::call(root, 1, [0; 3]).is_ok() || ep::call(client, 1, [0; 3]) != Err(E_CLOSED) { return Err("destroyed endpoint still answers"); }
    ep::notify(OP_DESTROY, note, 0).map_err(|_| "destroy notification")?;
    Ok([("ipc2_roundtrip", n, trip), ("ipc2_signal_poll", n, signal)])
}
