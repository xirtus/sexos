// SPDX-License-Identifier: GPL-3.0-only
//! `wasm layout @FILE` (or `wasm layout spiral`): a window layout plugin's own run,
//! which the Shell starts for `silk layout WS plugin:...` (slice S4 follow-up). It
//! opens no window. It answers the Shell's [`OP_LAYOUT_ASK`] with the rectangles the
//! guest's `layout(n, w, h)` export wrote, one [`OP_LAYOUT_RECT`] call each, or one
//! [`OP_LAYOUT_FAIL`] saying why there are none. Each call to the guest is a fresh
//! instance with `FUEL_PER_CALL` fuel; the guest imports nothing.
//!
//! What the run can reach: Files binds it to the arguments Spindle prepared, so it may
//! read (and, Files' grants being whole, also write) the one file `@FILE` names and its
//! own folder `/data/wasm`; it has no network. The guest itself reaches none of that.
//!
//! Serial record: `[wasm.layout.ready] what=.. bytes=.. ok=0|1 reason=..`,
//! `[wasm.layout.answer] gen=.. n=.. w=.. h=.. ok=0|1 reason=..`.
use alloc::vec;
use sex_app::{files, mailbox};
use sex_pdx::wasm_layout::{self as wire, Ask, Fail, Rect, RectMsg, OP_LAYOUT_ASK, OP_LAYOUT_FAIL, OP_LAYOUT_RECT};
use sex_pdx::{app_lifecycle as lifecycle, PdxMessage};
use sex_wasm::layout::{LayoutError, LayoutLimits, LayoutPlugin};

/// `crates/sex-wasm/guests/layouts/spiral.wat`.
const SPIRAL: &[u8] = include_bytes!("../../../crates/sex-wasm/guests/layouts/spiral.wasm");
const SHELL: u32 = 3;

fn load(what: &[u8]) -> Result<LayoutPlugin, Fail> {
    let limits = LayoutLimits { fuel_per_call: wire::FUEL_PER_CALL, memory_bytes: wire::MEMORY_BYTES, max_rects: wire::MAX_RECTS };
    let bytes = match what {
        b"spiral" => SPIRAL.to_vec(),
        _ if what.first() == Some(&b'@') => {
            let mut bytes = vec![0u8; wire::MODULE_BYTES + 1];
            match files::direct::read_at(what, 0, &mut bytes) {
                Ok(n) if n <= wire::MODULE_BYTES => { bytes.truncate(n); bytes }
                Ok(_) => { sex_pdx::serial_record!("[wasm.layout.module] too_large=1"); return Err(Fail::Module); }
                Err(e) => { sex_pdx::serial_record!("[wasm.layout.module] read_status={}", e); return Err(Fail::Read); }
            }
        }
        _ => return Err(Fail::Read),
    };
    LayoutPlugin::new(&bytes, limits).map_err(|e| {
        if let LayoutError::Module(why) = &e { sex_pdx::serial_record!("[wasm.layout.module] {}", why); }
        Fail::Module
    })
}

/// One call to the Shell, waiting briefly while its inbox is full.
fn tell(op: u64, w: [u64; 3]) -> bool {
    for _ in 0..2000 {
        match sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL, op, w[0], w[1], w[2]) {
            Ok(_) => return true,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
            Err(_) => return false,
        }
    }
    false
}

fn answer(plugin: &Result<LayoutPlugin, Fail>, ask: Ask) {
    let result = plugin.as_ref().map_err(|f| *f).and_then(|p| p.layout(ask.count, ask.w, ask.h).map_err(|e| match e {
        LayoutError::OutOfFuel => Fail::Fuel,
        LayoutError::Trapped(why) => { sex_pdx::serial_record!("[wasm.layout.trap] {}", why); Fail::Trap }
        LayoutError::Module(_) => Fail::Module,
        LayoutError::BadResult | LayoutError::BadCount => Fail::Result,
    }));
    let reason = result.as_ref().err().map_or("-", |f| f.name());
    let sent = match &result {
        Ok(rects) => rects.iter().enumerate().all(|(i, r)| {
            // Values that do not fit go as they are, cut to 32 bits: the Shell refuses them.
            let rect = Rect { x: r[0] as i32, y: r[1] as i32, w: r[2] as u32, h: r[3] as u32 };
            tell(OP_LAYOUT_RECT, RectMsg { generation: ask.generation, index: i as u16, count: ask.count as u16, rect }.wire())
        }),
        Err(f) => tell(OP_LAYOUT_FAIL, f.wire(ask.generation)),
    };
    sex_pdx::serial_record!("[wasm.layout.answer] gen={} n={} w={} h={} ok={} reason={} sent={}",
        ask.generation, ask.count, ask.w, ask.h, result.is_ok() as u8, reason, sent as u8);
}

pub fn serve(args: &[u8]) -> ! {
    let me = lifecycle::catalog(lifecycle::SELF_CATALOG_SLOT).map_or((0, 0), |i| (i.target_pd, i.task_id));
    let what = args.split(|b| *b == b' ').filter(|w| !w.is_empty()).nth(1).unwrap_or(b"");
    let plugin = load(what);
    sex_pdx::serial_record!("[wasm.layout.ready] target={} task={} what={} ok={} reason={}", me.0, me.1,
        core::str::from_utf8(what).unwrap_or("?"), plugin.is_ok() as u8, plugin.as_ref().err().map_or("-", |f| f.name()));
    let ours = |m: &PdxMessage| m.caller_pd == SHELL;
    loop {
        let mut latest: Option<Ask> = None;
        let mut take = |m: PdxMessage| -> bool {
            if lifecycle::close_requested(m.type_id, m.caller_pd, [m.arg0, m.arg1, m.arg2], me.0, me.1) { return true; }
            if m.type_id == OP_LAYOUT_ASK {
                // Only the newest ask matters: an older one would be dropped as stale.
                if let Some(a) = Ask::decode([m.arg0, m.arg1, m.arg2]) { latest = Some(a); }
            }
            false
        };
        let mut quit = false;
        while let Some(m) = mailbox::take(ours) { quit |= take(m); }
        while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
            if !ours(&m) { continue; }
            quit |= take(m);
        }
        if quit {
            sex_pdx::serial_record!("[wasm.layout.closed] target={} task={}", me.0, me.1);
            sex_pdx::task_finish(0);
        }
        match latest {
            Some(ask) => answer(&plugin, ask),
            // Sleep until the Shell says something.
            None => { let _ = sex_pdx::sys_wait_self_ticks(1_000); }
        }
    }
}
