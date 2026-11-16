// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `tray_items.rs` (slice S2): program tray items and
//! notification actions and inline replies. Programs' `OP_TRAY_ITEM` and
//! `OP_NOTIFY_ACTIONS` come in here; Bell's notifications are paired with
//! actions here; clicks, the pointer and keys come in through hook lines in
//! main.rs; Display is sent what to draw (`OP_TRAY_VIEW`) and programs what
//! was chosen (`OP_TRAY_EVENT`), both as plain calls (no answer is owed to the
//! Shell, so Bell's and sexnet's answers are never in the way).

use crate::tray_items::{Key, Out, Press, Screen, Tray};
use sex_pdx::app_catalog;
use sex_pdx::program_tray::{self as pt, ActionsMsg, ItemMsg, Section};

static mut TRAY: Tray = Tray::new();
static mut LAST_LOOK: u64 = 0;
static mut LAST_POINTER: (i32, i32) = (-1, -1);
/// When a send to Display or a program was first refused, while it waits.
static mut STUCK_SINCE: Option<u64> = None;

fn tray() -> &'static mut Tray { unsafe { &mut *core::ptr::addr_of_mut!(TRAY) } }

/// The panel and desktop as the Shell lays them out.
unsafe fn screen() -> Option<Screen> {
    use silkbar_model::ModuleSlot;
    let m = crate::chrome_metrics();
    let (w, h) = (crate::desktop_width().max(0) as usize, crate::desktop_height().max(0) as usize);
    let target = m.px(sex_pdx::desktop_style::chrome::TARGET) as usize;
    let mut bar = silkbar_model::DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(&mut bar, w, m.bar() as usize, target) { return None; }
    let s = m.scale().max(1) as usize;
    let chips = &bar.layout[ModuleSlot::Chip0 as usize];
    let left = bar.layout[1..1 + silkbar_model::WORKSPACE_COUNT].iter().filter(|b| b.w > 0).map(|b| b.x + b.w).max().unwrap_or(0) + 8 * s;
    Some(Screen { width: w, height: h, bar: m.bar() as usize, scale: s, right: chips.x, left, y: chips.y, size: target, gap: 4 * s })
}

fn alive(pd: u32, task: u32) -> bool { crate::running_task(pd) == Some(task) }

/// A program's `OP_TRAY_ITEM` or `OP_NOTIFY_ACTIONS`. True when it was one.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    let item = msg.type_id == pt::OP_TRAY_ITEM;
    if !item && msg.type_id != pt::OP_NOTIFY_ACTIONS { return false; }
    let (pd, task) = (msg.caller_pd, msg.caller_task);
    // Only a program's live run, by the kernel's caller stamp.
    if app_catalog::by_target(pd).is_none() || !alive(pd, task) {
        sex_pdx::serial_record!("[shell.tray_items.reject] pd={} task={} op={:#x} reason=not_a_live_program", pd, task, msg.type_id);
        return true;
    }
    let now = sex_pdx::get_ticks();
    if item {
        let Some(m) = ItemMsg::decode(msg.arg0, msg.arg1, msg.arg2) else {
            sex_pdx::serial_record!("[shell.tray_items.reject] pd={} task={} reason=bad_item_word", pd, task);
            return true;
        };
        let ok = tray().item_msg(pd, task, m);
        match m {
            ItemMsg::Commit { id } => {
                if let Some(i) = tray().items().find(|i| i.pd == pd && i.id == id) {
                    sex_pdx::serial_record!("[shell.tray_items.item] pd={} task={} id={} label={} rows={} tip={} ok={}", pd, task, id,
                        core::str::from_utf8(&i.label[..i.label_len as usize]).unwrap_or("?"), i.rows,
                        core::str::from_utf8(i.tip.as_bytes()).unwrap_or("?"), ok as u8);
                }
            }
            ItemMsg::Remove { id } => { sex_pdx::serial_record!("[shell.tray_items.remove] pd={} task={} id={} ok={}", pd, task, id, ok as u8); }
            _ if !ok => { sex_pdx::serial_record!("[shell.tray_items.reject] pd={} task={} reason=refused", pd, task); }
            _ => {}
        }
    } else {
        let Some(m) = ActionsMsg::decode(msg.arg0, msg.arg1, msg.arg2) else {
            sex_pdx::serial_record!("[shell.tray_items.reject] pd={} task={} reason=bad_actions_word", pd, task);
            return true;
        };
        let ok = tray().actions_msg(pd, task, m, now);
        if m == ActionsMsg::Commit || !ok { sex_pdx::serial_record!("[shell.tray_items.actions] pd={} task={} committed={} ok={}", pd, task, (m == ActionsMsg::Commit) as u8, ok as u8); }
    }
    flush();
    true
}

/// Bell forwarded a notification (from `notification_receive`): a card when
/// its sender gave actions for it.
pub unsafe fn notification(text: &sex_pdx::notify::Text) {
    let raw = sex_pdx::system_stats::name(text.sender as u32);
    let name = &raw[..raw.iter().position(|&c| c == 0).unwrap_or(raw.len())];
    let from = if name.is_empty() { crate::notify_sender_name(text.sender).as_bytes() } else { name };
    let Some((tag, queued)) = tray().notification(text.sender as u32, text.as_bytes(), from, sex_pdx::get_ticks()) else { return };
    sex_pdx::serial_record!("[shell.tray_items.paired] id={} pd={} tag={} queued={}", text.id, text.sender, tag, queued);
    flush();
}

/// A left press, before the decorations and windows see it. True when taken.
pub unsafe fn press(px: i32, py: i32) -> bool {
    // The window menu is modal while it shows.
    if crate::decorations_glue::menu_shown() { return false; }
    let Some(s) = screen() else { return false };
    let r = tray().press(&s, px.max(0) as usize, py.max(0) as usize, sex_pdx::get_ticks());
    match r {
        Press::Pass => return false,
        Press::Taken => {}
        Press::MenuOpened { pd, id, rows, panel, row_h, pad } => {
            // One menu at a time: the quick settings or calendar closes.
            if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None { crate::tray_close(); }
            sex_pdx::serial_record!("[shell.tray_items.menu] open=1 pd={} id={} rows={} x={} y={} w={} h={} row_h={} pad={}",
                pd, id, rows, panel.0, panel.1, panel.2, panel.3, row_h, pad);
        }
        Press::MenuClosed => { sex_pdx::serial_record!("[shell.tray_items.menu] open=0 reason=outside"); }
        Press::Chose { pd, id, row } => { sex_pdx::serial_record!("[shell.tray_items.menu] open=0 reason=chose pd={} id={} row={}", pd, id, row); }
        Press::Activated { pd, id } => { sex_pdx::serial_record!("[shell.tray_items.activate] pd={} id={}", pd, id); }
        Press::CardAction { pd, tag, action } => { sex_pdx::serial_record!("[shell.tray_items.card.answer] pd={} tag={} action={}", pd, tag, action); }
        Press::CardClosed { pd, tag } => { sex_pdx::serial_record!("[shell.tray_items.card.answer] pd={} tag={} closed=1", pd, tag); }
        Press::ReplyFocused { pd, tag } => { sex_pdx::serial_record!("[shell.tray_items.reply] focus=1 pd={} tag={}", pd, tag); }
        Press::ReplySent { pd, tag, len } => { sex_pdx::serial_record!("[shell.tray_items.reply] sent=1 pd={} tag={} len={} via=button", pd, tag, len); }
    }
    flush();
    true
}

/// Every key, before the desktop's own keys and programs (hooked next to
/// `access_glue::key`). True when taken: typing into an active reply field,
/// and Esc closing an item menu.
pub unsafe fn key(code: u8, value: u64) -> bool {
    let k = tray().key(code, value, sex_pdx::get_ticks());
    match k {
        Key::Pass => return false,
        Key::Sent { pd, tag, len } => { sex_pdx::serial_record!("[shell.tray_items.reply] sent=1 pd={} tag={} len={} via=enter", pd, tag, len); }
        Key::Left => { sex_pdx::serial_record!("[shell.tray_items.reply] focus=0 reason=escape"); }
        Key::MenuClosed => { sex_pdx::serial_record!("[shell.tray_items.menu] open=0 reason=escape"); }
        Key::Typed => {
            static mut TYPED_BUDGET: u32 = 64;
            if TYPED_BUDGET > 0 { TYPED_BUDGET -= 1; sex_pdx::serial_record!("[shell.tray_items.reply.typed] code={:#x} len={}", code, tray().reply_text().len()); }
        }
        Key::Taken => {}
    }
    flush();
    true
}

/// Display started again: it has to be told everything.
pub fn display_restarted() { tray().mark_all(); }

/// Whether a send waits to be retried soon (keeps the main loop from blocking).
pub unsafe fn busy() -> bool {
    STUCK_SINCE.is_some_and(|t| sex_pdx::get_ticks().saturating_sub(t) < 2_000)
}

/// Main loop: cards time out, ended programs' items go, the pointer
/// highlights menu rows and shows tooltips, and waiting sends go out.
pub unsafe fn poll() {
    let now = sex_pdx::get_ticks();
    if let Some((pd, tag)) = tray().tick(now) { sex_pdx::serial_record!("[shell.tray_items.card.answer] pd={} tag={} expired=1", pd, tag); }
    if now.saturating_sub(LAST_LOOK) >= 250 {
        LAST_LOOK = now;
        let gone = tray().retain(alive, now);
        if gone != 0 { sex_pdx::serial_record!("[shell.tray_items.cleanup] gone={} items={} cards={}", gone, tray().items().count(), tray().queued()); }
    }
    let s = screen();
    let p = (crate::POINTER_X, crate::POINTER_Y);
    // A new display scale or resolution moves everything: Display is sent
    // it all again, laid out as presses will find it.
    let moved = s.is_some_and(|s| tray().set_screen(&s));
    if moved { sex_pdx::serial_record!("[shell.tray_items.screen] relayout=1 width={} height={} scale={}", s.unwrap().width, s.unwrap().height, s.unwrap().scale); }
    if p != LAST_POINTER || moved {
        LAST_POINTER = p;
        if let Some(s) = s { tray().pointer(&s, p.0.max(0) as usize, p.1.max(0) as usize); }
    }
    flush();
}

/// Sends what changed to Display and what was chosen to programs.
unsafe fn flush() {
    let t = tray();
    let mut stuck = false;
    if t.dirty() != 0 {
        if let Some(s) = screen() {
            let view = t.view(&s);
            for section in Section::ALL {
                if t.dirty() & section.bit() == 0 { continue; }
                let ok = pt::view_calls(&view, section, |w| crate::pdx_call(sex_pdx::SLOT_DISPLAY, pt::OP_TRAY_VIEW, w[0], w[1], w[2]).0 == 0);
                if !ok { stuck = true; break; }
                t.sent(section);
                view_receipt(&s, &view, section);
            }
        }
    }
    while let Some(out) = t.next_out() {
        let (pd, task) = (out.pd(), out.task());
        let route = app_catalog::by_target(pd).map(|r| r.route_slot).filter(|_| alive(pd, task));
        let Some(route) = route else {
            sex_pdx::serial_record!("[shell.tray_items.told] pd={} task={} ok=0 reason=gone", pd, task);
            t.told();
            continue;
        };
        let mut status = 0;
        let mut send = |e: pt::Event| { if status == 0 { let w = e.wire(); status = crate::pdx_call(route, pt::OP_TRAY_EVENT, w[0], w[1], w[2]).0; } };
        match out {
            Out::Event { event, .. } => send(event),
            Out::Reply { tag, text, .. } => for e in pt::reply_events(tag, text.as_bytes()) { send(e); },
        }
        if status == sex_pdx::ERR_SERVICE_NOT_READY { stuck = true; break; }
        match out {
            Out::Event { event, .. } => { sex_pdx::serial_record!("[shell.tray_items.told] pd={} task={} event={:?} ok={}", pd, task, event, (status == 0) as u8); }
            Out::Reply { tag, text, .. } => { sex_pdx::serial_record!("[shell.tray_items.told] pd={} task={} reply tag={} len={} ok={}", pd, task, tag, text.len, (status == 0) as u8); }
        }
        t.told();
    }
    STUCK_SINCE = if stuck { Some(STUCK_SINCE.unwrap_or(sex_pdx::get_ticks())) } else { None };
}

unsafe fn view_receipt(s: &Screen, v: &pt::View, section: Section) {
    match section {
        Section::Icons => {
            static mut ICON_BUDGET: u32 = 64;
            for (slot, ic) in v.icons().iter().enumerate() {
                if ICON_BUDGET == 0 { break; }
                ICON_BUDGET -= 1;
                sex_pdx::serial_record!("[shell.tray_items.icon] slot={} label={} x={} y={} size={}", slot,
                    core::str::from_utf8(&ic.label[..ic.label_len as usize]).unwrap_or("?"), ic.x, ic.y, ic.size);
            }
        }
        Section::Card => match &v.card {
            Some(c) => {
                let Some(l) = pt::CardLayout::new(s.width, s.bar, s.scale, c.actions as usize, c.reply) else { return };
                let mid = |r: pt::Rect| (r.0 + r.2 / 2, r.1 + r.3 / 2);
                let (fx, fy) = l.field.map_or((0, 0), mid);
                let (sx, sy) = l.send.map_or((0, 0), mid);
                let (ax, ay) = if l.action_count > 0 { mid(l.actions[0]) } else { (0, 0) };
                sex_pdx::serial_record!("[shell.tray_items.card] shown=1 actions={} reply={} x={} y={} w={} h={} action0={},{} field={},{} send={},{} close={},{} summary={}",
                    c.actions, c.reply as u8, l.panel.0, l.panel.1, l.panel.2, l.panel.3, ax, ay, fx, fy, sx, sy, mid(l.close).0, mid(l.close).1,
                    core::str::from_utf8(c.summary()).unwrap_or("?"));
            }
            None => { sex_pdx::serial_record!("[shell.tray_items.card] shown=0"); }
        },
        _ => {}
    }
}
