//! Private read-only embed layout probe. Never merge into the production browser.
use super::sex_html::{dom::Dom, style::Style};
use super::Browser;
use alloc::string::String;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static STATES: [AtomicU32; 2] = [const { AtomicU32::new(0) }; 2];
static LAYOUTS: [AtomicU32; 2] = [const { AtomicU32::new(0) }; 2];
static SHOWS: AtomicU32 = AtomicU32::new(0);
static LAST_LAYOUT: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static LAYOUT_REPORTS: [AtomicU32; 2] = [const { AtomicU32::new(0) }; 2];

fn text(s: &str) -> String {
    s.chars()
        .take(192)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}
fn ordinal(b: &Browser) -> usize {
    b.in_frame.is_some() as usize
}

pub(super) fn state(b: &mut Browser, phase: &str) {
    let seq = STATES[ordinal(b)].fetch_add(1, Ordering::Relaxed) + 1;
    if seq > 128 && phase != "final" {
        return;
    }
    let view = b.view;
    let window = b.window.size();
    let frame = b.in_frame.map_or(-1, |n| n as i64);
    let doc = b.doc.get();
    let connected = doc.dom.descendants(Dom::ROOT).len();
    sex_pdx::serial_record!("[web.private.state] local_seq={} phase={} frame={} window={}x{} view={}x{} doc_view={}x{} nodes={} connected={} dirty={} items={} height={} frame_dirty={} frames_check={} full_redraw={} timer={} classic={} fetches={} modules={} url={}",
        seq, phase, frame, window.0, window.1, view.0, view.1, doc.viewport.0, doc.viewport.1,
        doc.dom.nodes.len(), connected, doc.dirty as u8, b.page.items.len(), b.page.height,
        b.frame_dirty as u8, b.frames_check as u8, b.full_redraw as u8,
        b.timer.map_or(-1, |t| t as i64), doc.classic_scripts.len(), doc.fetches.len(), doc.modules_wanted.len(), text(&b.url));
}

pub(super) fn layout(b: &mut Browser, styles: &[Option<Style>]) {
    let seq = LAYOUTS[ordinal(b)].fetch_add(1, Ordering::Relaxed) + 1;
    let context = ordinal(b);
    let now = sex_pdx::get_ticks();
    if seq > 2 && now.saturating_sub(LAST_LAYOUT[context].load(Ordering::Relaxed)) < 30_000 {
        return;
    }
    if LAYOUT_REPORTS[context].fetch_add(1, Ordering::Relaxed) >= 8 {
        return;
    }
    LAST_LAYOUT[context].store(now, Ordering::Relaxed);
    state(b, "layout");
    let frame = b.in_frame.map_or(-1, |n| n as i64);
    let doc = b.doc.get();
    let connected = doc.dom.descendants(Dom::ROOT);
    let mut chosen = alloc::vec::Vec::new();
    for &n in &connected {
        let tag = doc.dom.tag(n).unwrap_or("");
        let id = doc.dom.attr(n, "id").unwrap_or("");
        if !matches!(tag, "html" | "body" | "iframe" | "video" | "audio") && !id.contains("player")
        {
            continue;
        }
        let mut parent = Some(n);
        for _ in 0..32 {
            let Some(p) = parent else { break };
            if !chosen.contains(&p) {
                chosen.push(p);
            }
            parent = doc.dom.nodes[p].parent;
        }
        if chosen.len() >= 64 {
            break;
        }
    }
    for n in connected.into_iter().take(12) {
        if !chosen.contains(&n) {
            chosen.push(n);
        }
    }
    let mut shown = 0;
    for n in chosen {
        let Some(tag) = doc.dom.tag(n) else { continue };
        let id = doc.dom.attr(n, "id").unwrap_or("");
        let Some(style) = styles.get(n).and_then(Option::as_ref) else {
            continue;
        };
        let rect = b.page.boxes.get(n).copied().flatten();
        sex_pdx::serial_record!("[web.private.node] layout_seq={} frame={} node={} parent={} tag={} id={:?} class={:?} style={:?} children={} box={:?} display={:?} hidden={} opacity={} position={:?} width={:?} height={:?} min_height={:?} overflow_clip={} color={:x} background={:x}",
            seq, frame, n, doc.dom.nodes[n].parent.map_or(-1, |p| p as i64), tag, text(id), text(doc.dom.attr(n,"class").unwrap_or("")), text(doc.dom.attr(n,"style").unwrap_or("")), doc.dom.nodes[n].children.len(), rect,
            style.display, style.hidden as u8, style.opacity, style.position, style.width, style.height, style.min_height, style.overflow_clip as u8, style.color, style.background);
        shown += 1;
        if shown >= 48 {
            break;
        }
    }
    for f in &b.frames {
        sex_pdx::serial_record!("[web.private.frame] layout_seq={} node={} rect={:?} view={}x{} items={} height={} url={}",
            seq, f.node, f.rect, f.view.0, f.view.1, f.page.items.len(), f.page.height, text(&f.url));
    }
}

pub(super) fn publication(b: &mut Browser) {
    let seq = SHOWS.fetch_add(1, Ordering::Relaxed) + 1;
    if seq > 96 {
        return;
    }
    let window = b.window.size();
    // This is a client request receipt, not an invented Display generation.
    // Correlate [display.app.pixels] and actual screenshots for admission/visibility.
    sex_pdx::serial_record!("[web.private.publication] local_seq={} target={} task={} window={}x{} requested={}x{} view={}x{} frame_count={} items={} full_redraw={} pixel_generation=unavailable",
        seq, b.window.target, b.window.task, window.0, window.1, b.surface.w, b.surface.h, b.view.0, b.view.1, b.frames.len(), b.page.items.len(), b.full_redraw as u8);
}

static MESSAGES: AtomicU32 = AtomicU32::new(0);
pub(super) fn message(direction: &str, node: usize, payload: &str) {
    let seq = MESSAGES.fetch_add(1, Ordering::Relaxed) + 1;
    if seq > 96 {
        return;
    }
    sex_pdx::serial_record!(
        "[web.private.message] local_seq={} direction={} node={} bytes={} payload_prefix={:?}",
        seq,
        direction,
        node,
        payload.len(),
        text(payload)
    );
}

static FINAL_REPORTS: AtomicU32 = AtomicU32::new(0);
/// A native-only diagnostic key reads current fields without evaluating JS,
/// recomputing CSS/layout, draining jobs or publishing pixels.
pub(super) fn final_snapshot(b: &mut Browser) {
    if FINAL_REPORTS.fetch_add(1, Ordering::Relaxed) >= 4 {
        return;
    }
    state(b, "final");
    raw(b.doc.get(), &b.page, -1);
    for f in &mut b.frames {
        sex_pdx::serial_record!(
            "[web.private.final.frame] node={} rect={:?} view={}x{} items={} height={}",
            f.node,
            f.rect,
            f.view.0,
            f.view.1,
            f.page.items.len(),
            f.page.height
        );
        raw(f.doc.get(), &f.page, f.node as i64);
    }
}
fn raw(doc: &sex_js::Document, page: &super::sex_html::layout::Page, frame: i64) {
    sex_pdx::serial_record!("[web.private.final.dom] frame={} nodes={} dirty={} viewport={}x{} items={} width={} height={} background={:x} styles_recomputed=0 js_evaluated=0",frame,doc.dom.nodes.len(),doc.dirty as u8,doc.viewport.0,doc.viewport.1,page.items.len(),page.width,page.height,page.background);
    let mut nout = 0;
    for n in doc.dom.descendants(Dom::ROOT) {
        let Some(tag) = doc.dom.tag(n) else { continue };
        let id = doc.dom.attr(n, "id").unwrap_or("");
        if !matches!(tag, "html" | "body" | "iframe" | "video" | "audio") && !id.contains("player")
        {
            continue;
        }
        sex_pdx::serial_record!("[web.private.final.node] frame={} node={} tag={} id={:?} class={:?} style={:?} children={} box={:?}",frame,n,tag,text(id),text(doc.dom.attr(n,"class").unwrap_or("")),text(doc.dom.attr(n,"style").unwrap_or("")),doc.dom.nodes[n].children.len(),page.boxes.get(n).copied().flatten());
        nout += 1;
        if nout >= 48 {
            break;
        }
    }
}
